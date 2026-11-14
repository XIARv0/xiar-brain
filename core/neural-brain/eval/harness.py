# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""NeuralBrain evaluation harness (NeuralBrain.md section 7). Real numbers only.

Metrics per variant, after every simulated day (= one idle training window):
  recall_trained   outcome-label accuracy on all lessons taught so far (exact wording)
  recall_day1      the same on day-1 lessons only -> forgetting curve
  heldout_para     accuracy on held-out paraphrases of taught lessons (never trained wording)
  avoid_mistake    share of failed/rejected actions, replayed in a NEW context with search unable to
                   see the lesson, where the brain alone raises a gate-strength signal
  false_alarm      share of never-taught actions that raise a gate-strength fail/rejected signal
  why_exact        share of taught failures whose 'why' the brain regenerates exactly
Pipeline check (main variant): lessons go through the real service -> sealed store -> trainer;
plaintext search over every file in the data dir must find 0 lesson texts.

Usage: python -m eval.harness [--days 6] [--variants smf_replay,dense_replay,dense_noreplay,smf_noreplay]
"""
from __future__ import annotations

import argparse
import json
import pathlib
import random
import sys
import tempfile
import time

PKG = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PKG))

import torch  # noqa: E402

from xiar_brain.brain import Brain  # noqa: E402
from xiar_brain.config import load_config  # noqa: E402
from eval.synthetic import build, novel_actions, paraphrase, split_days  # noqa: E402

VARIANTS = {
    "smf_replay": {},
    "smf_noreplay": {"train.replay_ratio": 0.0},
    "dense_replay": {"smf.enabled": False},
    "dense_noreplay": {"smf.enabled": False, "train.replay_ratio": 0.0},
}
GATE = {"fail", "rejected"}


def _strip(l):
    return {k: v for k, v in l.items() if not k.startswith("_")}


def evaluate(brain: Brain, taught, day1, novel, gate_conf: float, new_ctx: str, warn_conf: float = 0.6) -> dict:
    bad = [l for l in taught if l["outcome"] in GATE]
    avoid = warn = lab = 0
    for l in bad:
        s = brain.signals(new_ctx, l["action"], with_why=False)
        avoid += int(s["label"] in GATE and s["confidence"] >= gate_conf)
        warn += int(s["label"] in GATE and s["confidence"] >= warn_conf)
        lab += int(s["label"] in GATE)
    fa = fa_warn = 0
    for a in novel:
        s = brain.signals(new_ctx, a, with_why=False)
        fa += int(s["label"] in GATE and s["confidence"] >= gate_conf)
        fa_warn += int(s["label"] in GATE and s["confidence"] >= warn_conf)
    fails = [l for l in taught if l["outcome"] == "fail"]
    why_ok = 0
    for l in fails:
        p = brain.predict(l["context"], l["action"])
        why_ok += int(brain.generate_why(p["ids"], l["outcome"]).strip() == l["why"])
    para = [dict(l, action=paraphrase(l)) for l in taught if l.get("_tool")]
    return {"recall_trained": round(brain.recall(taught), 4), "recall_day1": round(brain.recall(day1), 4),
            "heldout_para": round(brain.recall(para), 4) if para else None,
            "avoid_mistake": round(avoid / max(1, len(bad)), 4), "avoid_warn": round(warn / max(1, len(bad)), 4),
            "avoid_label": round(lab / max(1, len(bad)), 4),
            "false_alarm": round(fa / max(1, len(novel)), 4), "false_alarm_warn": round(fa_warn / max(1, len(novel)), 4),
            "why_exact": round(why_ok / max(1, len(fails)), 4)}


def run_variant(cfg, name, days, novel, log):
    c = cfg.with_overrides(VARIANTS[name])
    torch.manual_seed(0); random.seed(0)
    b = Brain(c)
    taught, curve = [], []
    gate_conf = float(c.get("signals.tool_gate_confidence"))
    for d, lessons in enumerate(days, 1):
        t0 = time.time()
        info = b.train_window([_strip(l) for l in lessons], [_strip(l) for l in taught])
        taught += lessons
        m = evaluate(b, taught, days[0], novel, gate_conf, "session voice", float(c.get("signals.min_confidence")))
        m.update(day=d, secs=round(time.time() - t0, 1), smf=info["smf"], n_keys=[x.n_keys for x in b.model.memories()],
                 utilisation=info["utilisation"], loss=round(info["loss_last"], 4))
        curve.append(m)
        log(f"{name} day {d}: {json.dumps(m)}")
    peak = max(x["recall_day1"] for x in curve)
    return {"variant": name, "curve": curve, "final": curve[-1],
            "forgetting_day1": round(peak - curve[-1]["recall_day1"], 4), "params": b.model.n_params(),
            "device": str(b.device)}


def pipeline_check(cfg, lessons, log) -> dict:
    """Service -> sealed store -> trainer, then plaintext 'search' over the whole data dir."""
    from fastapi.testclient import TestClient
    from xiar_brain.sealed_store import generate_keys
    from xiar_brain.service import create_app
    from xiar_brain.trainer import run_window
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as td:
        root = pathlib.Path(td)
        c = cfg.with_overrides({k: str(root / pathlib.Path(v).relative_to("data")) if str(v).startswith("data") else v
                                for k, v in {f"paths.{k}": v for k, v in cfg.section("paths").items()}.items()})
        generate_keys(c.path("paths.sealed_public_key"), c.path("paths.sealed_private_key"))
        app = TestClient(create_app(c))
        toks = json.loads(c.path("paths.tokens_file").read_text())
        H = lambda r: {"Authorization": f"Bearer {toks[r]}"}  # noqa: E731
        written = sum(app.post("/v1/lessons", json=_strip(l), headers=H("writer")).status_code == 201 for l in lessons)
        out = run_window(c, force=True, log=lambda *_: None)
        # "search": every byte of every file under the data dir, as any indexer/embedder would see it
        texts = {l["why"] for l in lessons if len(l["why"]) > 8} | {l["action"] for l in lessons}
        blob = b"".join(p.read_bytes() for p in root.rglob("*") if p.is_file() and p.suffix != ".pt")
        leaks = sum(1 for t in texts if t.encode() in blob)
        app2 = TestClient(create_app(c.with_overrides({"service.reload_poll_seconds": 0})))
        probe = [l for l in lessons if l["outcome"] == "fail"][:10]
        hits = 0
        for l in probe:
            r = app2.post("/v1/signals", json={"action": l["action"], "context": "session voice"}, headers=H("query")).json()
            hits += int(bool(r["signals"]) and r["signals"][0]["label"] == "fail")
        res = {"lessons_written": written, "trainer": {k: out.get(k) for k in ("ran", "version", "promoted", "reason", "device", "recall_all")},
               "plaintext_hits_in_data_dir": leaks, "texts_checked": len(texts),
               "api_fail_signal_hits": f"{hits}/{len(probe)}"}
        log("pipeline: " + json.dumps(res))
        return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--config"); ap.add_argument("--days", type=int, default=6)
    ap.add_argument("--variants", default=",".join(VARIANTS)); ap.add_argument("--out")
    ap.add_argument("--set", action="append", default=[], help="override key=toml_literal")
    a = ap.parse_args()
    cfg = load_config(a.config)
    import tomllib
    ov = {kv.split("=", 1)[0]: tomllib.loads("x=" + kv.split("=", 1)[1])["x"] for kv in a.set}
    lessons = build()
    days = split_days(lessons, a.days)
    # newborn: day 1 is the dense bootstrap, every later day is a sparse (SMF) update - unless overridden
    cfg = cfg.with_overrides({"smf.bootstrap_lessons": len(days[0]), **ov})
    novel = novel_actions(lessons)
    log = lambda s: print(s, flush=True)  # noqa: E731
    log(f"lessons={len(lessons)} days={a.days} per_day~{len(days[0])} novel={len(novel)} tokenizer={cfg.get('tokenizer.name')}")
    report = {"config_overrides": ov, "lessons": len(lessons), "days": a.days, "variants": [],
              "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z")}
    for v in a.variants.split(","):
        report["variants"].append(run_variant(cfg, v, days, novel, log))
    report["pipeline"] = pipeline_check(cfg, lessons, log)
    out = pathlib.Path(a.out) if a.out else cfg.path("paths.eval_reports_dir") / f"eval-{time.strftime('%Y%m%d-%H%M%S')}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2), encoding="utf-8")
    log(f"report -> {out}")


if __name__ == "__main__":
    main()
