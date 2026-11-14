# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Training job. The ONLY component that opens the sealed store for reading (role=trainer).

Runs in idle windows: GPU must have >= train.idle_min_free_vram_gb free and utilisation
<= train.idle_max_gpu_util (Hermes/LLM priority). Eval-gated promotion; failed candidates are kept
but not served.
"""
from __future__ import annotations

import json
import os
import pathlib
import random
import subprocess
import time

import torch

from .brain import Brain, pick_device
from .checkpoints import CheckpointStore
from .config import BrainConfig
from .sealed_store import SealedReader


def gpu_idle(cfg: BrainConfig) -> tuple[bool, str]:
    if not torch.cuda.is_available():
        return (bool(cfg.get("train.allow_cpu_fallback", True)), "no CUDA; cpu fallback" )
    free, total = torch.cuda.mem_get_info()
    need = float(cfg.get("train.idle_min_free_vram_gb")) * 2**30
    if free < need:
        return False, f"free VRAM {free/2**30:.1f} GiB < {need/2**30:.1f} GiB"
    try:
        out = subprocess.run(["nvidia-smi", "--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"],
                             capture_output=True, text=True, timeout=10).stdout.strip().splitlines()
        util = int(out[0]) if out else 0
    except Exception:
        util = 0
    if util > int(cfg.get("train.idle_max_gpu_util")):
        return False, f"GPU util {util}% > {cfg.get('train.idle_max_gpu_util')}%"
    return True, f"idle (free {free/2**30:.1f} GiB, util {util}%)"


class _Lock:
    def __init__(self, path: pathlib.Path):
        self.path = path

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        try:
            self.fd = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            if time.time() - self.path.stat().st_mtime > 6 * 3600:
                self.path.unlink(missing_ok=True)
                self.fd = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            else:
                raise RuntimeError("another training window is running")
        os.write(self.fd, str(os.getpid()).encode())
        return self

    def __exit__(self, *a):
        os.close(self.fd)
        self.path.unlink(missing_ok=True)


def _state(cfg) -> dict:
    p = cfg.path("paths.state_file")
    return json.loads(p.read_text(encoding="utf-8")) if p.exists() else {"trained_seq": 0}


def _save_state(cfg, st):
    p = cfg.path("paths.state_file")
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(st, indent=2), encoding="utf-8")


def gate(cfg: BrainConfig, cand: Brain, prev: Brain | None, new: list[dict], old: list[dict], first: bool) -> tuple[bool, dict, str]:
    all_l = old + new
    m = {"recall_all": cand.recall(all_l), "recall_new": cand.recall(new)}
    if old:
        m["recall_old"] = cand.recall(old)
        if prev is not None:
            m["recall_old_prev"] = prev.recall(old)
            m["forgetting"] = max(0.0, m["recall_old_prev"] - m["recall_old"])
    if first and cfg.get("eval.promote_first_always", True):
        return True, m, "first checkpoint (newborn)"
    if m["recall_all"] < float(cfg.get("eval.promote_min_recall")):
        return False, m, "recall below eval.promote_min_recall"
    if m.get("forgetting", 0.0) > float(cfg.get("eval.max_forgetting")):
        return False, m, "forgetting above eval.max_forgetting"
    return True, m, "passed gates"


def run_window(cfg: BrainConfig, force: bool = False, log=print) -> dict:
    ok, why = gpu_idle(cfg)
    if not ok and not force:
        return {"ran": False, "reason": why}
    with _Lock(cfg.path("paths.lock_file")):
        reader = SealedReader(cfg.path("paths.sealed_db"), cfg.path("paths.sealed_private_key"), role="trainer")
        st = _state(cfg)
        lessons = list(reader.iter_lessons())
        reader.close()
        new = [l for l in lessons if l["_seq"] > st["trained_seq"]]
        old = [l for l in lessons if l["_seq"] <= st["trained_seq"]]
        if len(new) < int(cfg.get("train.min_new_lessons")):
            return {"ran": False, "reason": "no new lessons"}
        ck = CheckpointStore(cfg.path("paths.checkpoints_dir"))
        device = pick_device(cfg)
        pp = ck.promoted_path()
        prev = Brain.load(cfg, pp, device) if pp else None
        cand = Brain.load(cfg, pp, device) if pp else Brain(cfg, device=device)
        deadline = time.time() + 60 * float(cfg.get("train.max_window_minutes"))
        info = cand.train_window(new, old, deadline=deadline)
        promote, metrics, reason = gate(cfg, cand, prev, new, old, first=pp is None)
        v = ck.next_version()
        cand.save(ck.path(v))
        ck.record(v, promote, {**metrics, **{k: info[k] for k in ("steps", "loss_last", "smf", "grown", "utilisation")}}, reason)
        if promote:
            st["trained_seq"] = max(l["_seq"] for l in new)
            _save_state(cfg, st)
        out = {"ran": True, "version": v, "promoted": promote, "reason": reason, "device": str(device),
               "idle": why, "new": len(new), "old": len(old), **metrics, "train": info}
        log(json.dumps(out, default=str))
        return out
