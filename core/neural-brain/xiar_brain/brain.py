# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""The XIAR brain: training (dense bootstrap -> sparse memory finetuning + replay + growth) and signals."""
from __future__ import annotations

import json
import math
import pathlib
import random
import time
from dataclasses import dataclass, field

import torch
import torch.nn.functional as F

from .config import BrainConfig
from .encoding import LABEL_TOKENS, encode_lesson, encode_query, label_ids
from .model import BrainModel
from .tokenizer import BrainTokenizer

SIGNAL_TEXT = {
    "fail": "failed before",
    "rejected": "owner rejected this before",
    "approved": "owner approved this before",
    "prefer": "owner prefers",
    "corrected": "owner corrected this before",
    "ok": "worked before",
}


def pick_device(cfg: BrainConfig) -> torch.device:
    want = str(cfg.get("train.device", "cuda"))
    if want.startswith("cuda") and torch.cuda.is_available():
        return torch.device(want)
    if want.startswith("cuda") and not cfg.get("train.allow_cpu_fallback", True):
        raise RuntimeError("CUDA requested but unavailable and train.allow_cpu_fallback=false")
    return torch.device("cpu")


@dataclass
class SmfState:
    bg_batches: int = 0
    bg_df: list = field(default_factory=list)      # per memory layer: LongTensor(n_slots)
    access: list = field(default_factory=list)     # per memory layer: LongTensor(n_slots) cumulative hits


class Brain:
    def __init__(self, cfg: BrainConfig, tok: BrainTokenizer | None = None, device: torch.device | None = None,
                 model: BrainModel | None = None):
        self.cfg = cfg
        self.tok = tok or BrainTokenizer.from_config(cfg)
        self.device = device or pick_device(cfg)
        self.model = (model or BrainModel.from_config(cfg, self.tok.vocab_size, self.tok.pad_id)).to(self.device)
        self.max_len = int(cfg.get("model.max_len"))
        self.labels = label_ids(self.tok)
        self.lessons_seen = 0
        self.windows = 0
        mems = self.model.memories()
        self.smf = SmfState(0, [torch.zeros(m.n_slots, dtype=torch.long) for m in mems],
                            [torch.zeros(m.n_slots, dtype=torch.long) for m in mems])

    # ---------------- persistence ----------------
    def state(self) -> dict:
        return {"hparams": self.model.hparams, "model": self.model.state_dict(),
                "smf": {"bg_batches": self.smf.bg_batches, "bg_df": self.smf.bg_df, "access": self.smf.access},
                "lessons_seen": self.lessons_seen, "windows": self.windows,
                "tokenizer": {"name": self.tok.name, "base_vocab": self.tok.base_vocab, "specials": self.tok.specials}}

    def save(self, path: pathlib.Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(".tmp")
        torch.save(self.state(), tmp)
        tmp.replace(path)

    @classmethod
    def load(cls, cfg: BrainConfig, path: pathlib.Path, device: torch.device | None = None) -> "Brain":
        st = torch.load(path, map_location="cpu", weights_only=False)
        tok = BrainTokenizer.from_config(cfg)
        if st["tokenizer"]["base_vocab"] != tok.base_vocab or st["tokenizer"]["specials"] != tok.specials:
            raise RuntimeError("checkpoint was born with a different tokenizer; re-birth required")
        hp = dict(st["hparams"]); mem = hp.pop("mem")
        model = BrainModel(**hp, mem=mem)
        model.load_state_dict(st["model"])
        b = cls(cfg, tok, device, model)
        b.smf = SmfState(st["smf"]["bg_batches"], st["smf"]["bg_df"], st["smf"]["access"])
        b.lessons_seen, b.windows = st["lessons_seen"], st["windows"]
        return b

    # ---------------- batching ----------------
    def _batch(self, lessons: list[dict]):
        encs = [encode_lesson(self.tok, l, self.max_len) for l in lessons]
        T = max(len(e.ids) for e in encs)
        ids = torch.full((len(encs), T), self.tok.pad_id, dtype=torch.long)
        w = torch.zeros((len(encs), T - 1))
        lw, ww = float(self.cfg.get("train.label_loss_weight")), float(self.cfg.get("train.why_loss_weight"))
        for i, e in enumerate(encs):
            ids[i, : len(e.ids)] = torch.tensor(e.ids)
            w[i, : len(e.ids) - 1] = 1.0
            w[i, e.label_pos - 1] = lw                      # predicting the label token
            w[i, e.why_start - 1: len(e.ids) - 1] = ww      # predicting why + eos
        return ids.to(self.device), w.to(self.device)

    # ---------------- training ----------------
    def _smf_active(self) -> bool:
        return bool(self.cfg.get("smf.enabled")) and self.lessons_seen >= int(self.cfg.get("smf.bootstrap_lessons"))

    def train_window(self, new: list[dict], old: list[dict], deadline: float | None = None, log=None) -> dict:
        """One idle-window training run on `new` lessons with `old` lessons replayed."""
        if not new:
            return {"steps": 0}
        cfg = self.cfg
        smf = self._smf_active()
        mems = self.model.memories()
        mem_params = [m.values.weight for m in mems]
        mem_ids = {id(p) for p in mem_params}
        other = [p for p in self.model.parameters() if id(p) not in mem_ids]
        freeze_other = smf and not cfg.get("smf.train_non_memory_after_bootstrap", False)
        for p in other:
            p.requires_grad_(not freeze_other)
        groups = [{"params": mem_params, "lr": float(cfg.get("train.memory_lr"))}]
        if not freeze_other:
            groups.append({"params": other, "lr": float(cfg.get("train.lr"))})
        opt = torch.optim.AdamW(groups, weight_decay=float(cfg.get("train.weight_decay")))
        bs = int(cfg.get("train.batch_size"))
        rr = float(cfg.get("train.replay_ratio"))
        n_new = max(1, int(round(bs * (1 - rr)))) if old else bs
        n_old = bs - n_new if old else 0
        top_t = int(cfg.get("smf.top_t"))
        clip = float(cfg.get("train.grad_clip"))
        self.model.train()
        for m in mems:
            m.record = True
        steps, losses, updated = 0, [], []
        for ep in range(int(cfg.get("train.epochs_per_window"))):
            order = new[:]
            random.shuffle(order)
            for s in range(0, len(order), n_new):
                if deadline and time.time() > deadline:
                    break
                batch = order[s: s + n_new] + (random.sample(old, min(n_old, len(old))) if n_old else [])
                ids, w = self._batch(batch)
                logits = self.model(ids[:, :-1])
                nll = F.cross_entropy(logits.float().reshape(-1, logits.size(-1)), ids[:, 1:].reshape(-1),
                                      reduction="none").view_as(w)
                loss = (nll * w).sum() / w.sum()
                opt.zero_grad(set_to_none=True)
                loss.backward()
                valid = ids[:, :-1].ne(self.tok.pad_id).reshape(-1)
                masks = []
                for li, m in enumerate(mems):
                    acc = m.last_indices[valid.to(m.last_indices.device)].reshape(-1).cpu()
                    counts = torch.bincount(acc, minlength=m.n_slots)
                    if smf:
                        tf = counts.float() / counts.sum().clamp(min=1)
                        idf = torch.log((1 + self.smf.bg_batches) / (1 + self.smf.bg_df[li].float())) + 1.0
                        score = tf * idf
                        k = min(top_t, int((counts > 0).sum()))
                        sel = torch.zeros(m.n_slots, dtype=torch.bool)
                        if k > 0:
                            sel[score.topk(k).indices] = True
                        masks.append(sel.to(self.device))
                        updated.append(int(k))
                    self.smf.bg_df[li] += (counts > 0).long()
                    self.smf.access[li] += counts
                self.smf.bg_batches += 1
                backups = []
                if smf:
                    for m, sel in zip(mems, masks):
                        m.values.weight.grad[~sel] = 0
                        backups.append(m.values.weight.data.clone())
                torch.nn.utils.clip_grad_norm_([p for g in groups for p in g["params"]], clip)
                opt.step()
                if smf:
                    with torch.no_grad():
                        for m, sel, bk in zip(mems, masks, backups):
                            m.values.weight.data[~sel] = bk[~sel]
                losses.append(loss.item())
                steps += 1
        for m in mems:
            m.record = False
            m.last_indices = None
        for p in self.model.parameters():
            p.requires_grad_(True)
        self.lessons_seen += len(new)
        self.windows += 1
        grown = self.maybe_grow()
        out = {"steps": steps, "loss_first": losses[0] if losses else None, "loss_last": losses[-1] if losses else None,
               "smf": smf, "avg_slots_updated": (sum(updated) / len(updated)) if updated else None,
               "grown": grown, "utilisation": self.utilisation()}
        if log:
            log(out)
        return out

    def utilisation(self) -> list[float]:
        return [round(float((a > 0).float().mean()), 4) for a in self.smf.access]

    def maybe_grow(self) -> list[int]:
        thr, step = float(self.cfg.get("memory.grow_threshold")), int(self.cfg.get("memory.grow_step"))
        cap, scale = int(self.cfg.get("memory.max_n_keys")), float(self.cfg.get("memory.grow_key_scale"))
        grown = []
        for li, m in enumerate(self.model.memories()):
            if self.utilisation()[li] >= thr and m.n_keys + step <= cap:
                K = m.n_keys
                m.grow(step, scale)
                for name in ("bg_df", "access"):
                    old = getattr(self.smf, name)[li]
                    new = torch.zeros(m.n_slots, dtype=torch.long)
                    new[m.remap_old_ids(torch.arange(K * K), K, m.n_keys)] = old
                    getattr(self.smf, name)[li] = new
                grown.append(li)
        if grown:
            self.model.hparams["mem"]["n_keys"] = [m.n_keys for m in self.model.memories()][0]
            ks = {m.n_keys for m in self.model.memories()}
            if len(ks) > 1:   # keep layers in lock-step so the checkpoint can be rebuilt from hparams
                tgt = max(ks)
                for li, m in enumerate(self.model.memories()):
                    if m.n_keys < tgt:
                        K = m.n_keys
                        m.grow(tgt - K, scale)
                        for name in ("bg_df", "access"):
                            old = getattr(self.smf, name)[li]
                            new = torch.zeros(m.n_slots, dtype=torch.long)
                            new[m.remap_old_ids(torch.arange(K * K), K, m.n_keys)] = old
                            getattr(self.smf, name)[li] = new
                self.model.hparams["mem"]["n_keys"] = tgt
        return grown

    # ---------------- inference ----------------
    @torch.no_grad()
    def _prompt(self, context: str, action: str):
        enc = encode_query(self.tok, context, action, self.max_len)
        ids = torch.tensor([enc.ids], device=self.device)
        logits = self.model(ids)[0].float()
        lp = F.log_softmax(logits, -1)
        a0, a1 = enc.action_span
        tgt = ids[0, a0:a1]
        nll = float(-lp[a0 - 1: a1 - 1].gather(-1, tgt.unsqueeze(-1)).mean()) if a1 > a0 else 99.0
        return enc, ids, lp[-1], nll

    @torch.no_grad()
    def predict(self, context: str, action: str) -> dict:
        self.model.eval()
        enc, ids, last, nll = self._prompt(context, action)
        names = list(self.labels)
        lab = torch.stack([last[self.labels[n]] for n in names])
        probs = F.softmax(lab, -1)
        best = int(probs.argmax())
        return {"label": names[best], "p": float(probs[best]), "probs": {n: round(float(p), 4) for n, p in zip(names, probs)},
                "action_nll": nll, "ids": ids, "label_name": names[best]}

    @torch.no_grad()
    def generate_why(self, ids: torch.Tensor, label: str) -> str:
        seq = torch.cat([ids, torch.tensor([[self.labels[label], self.tok.sid("<why>")]], device=self.device)], 1)
        out = []
        for _ in range(int(self.cfg.get("signals.gen_max_tokens"))):
            if seq.size(1) >= self.max_len:
                break
            nxt = int(self.model(seq)[0, -1].float().argmax())
            if nxt == self.tok.eos_id or nxt >= self.tok.base_vocab:
                break
            out.append(nxt)
            seq = torch.cat([seq, torch.tensor([[nxt]], device=self.device)], 1)
        return self.tok.decode(out)

    def familiarity(self, nll: float) -> float:
        t, s = float(self.cfg.get("signals.familiar_nll")), float(self.cfg.get("signals.familiar_scale"))
        return 1.0 / (1.0 + math.exp(-(t - nll) / s))

    def signals(self, context: str, action: str, with_why: bool = True) -> dict:
        pr = self.predict(context, action)
        fam = self.familiarity(pr["action_nll"])
        conf = pr["p"] * fam
        sig = {"label": pr["label"], "text": SIGNAL_TEXT[pr["label"]], "confidence": round(conf, 4),
               "label_p": round(pr["p"], 4), "familiarity": round(fam, 4), "probs": pr["probs"]}
        if with_why and conf >= float(self.cfg.get("signals.min_confidence")):
            sig["why"] = self.generate_why(pr["ids"], pr["label"])
        return sig

    @torch.no_grad()
    def recall(self, lessons: list[dict]) -> float:
        if not lessons:
            return float("nan")
        ok = sum(1 for l in lessons if self.predict(l.get("context", ""), l["action"])["label"] == l["outcome"])
        return ok / len(lessons)
