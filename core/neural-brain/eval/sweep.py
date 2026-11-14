# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Run the harness for several override sets; print a compact table. Usage: python -m eval.sweep OUTDIR"""
import json
import pathlib
import subprocess
import sys

SETS = {
    "default": [],
    "topt4000": ["smf.top_t=4000"],
    "ep300": ["train.epochs_per_window=300"],
    "topt4000_ep300": ["smf.top_t=4000", "train.epochs_per_window=300"],
    "memlr3e-2_topt4000": ["smf.top_t=4000", "train.memory_lr=3e-2"],
}


def main():
    out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
    cfg = sys.argv[2] if len(sys.argv) > 2 else None
    only = sys.argv[3].split(",") if len(sys.argv) > 3 else list(SETS)
    rows = []
    for name in only:
        rep = out / f"sweep-{name}.json"
        cmd = [sys.executable, "-m", "eval.harness", "--days", "6", "--out", str(rep)] + (["--config", cfg] if cfg else [])
        for s in SETS[name]:
            cmd += ["--set", s]
        subprocess.run(cmd, check=True, stdout=open(out / f"sweep-{name}.log", "w"), stderr=subprocess.STDOUT)
        r = json.loads(rep.read_text())
        for v in r["variants"]:
            f = v["final"]
            rows.append({"set": name, "variant": v["variant"], "recall": f["recall_trained"], "day1": f["recall_day1"],
                         "forget": v["forgetting_day1"], "para": f["heldout_para"], "avoid@gate": f["avoid_mistake"],
                         "avoid@warn": f["avoid_warn"], "avoid_label": f["avoid_label"], "fa@warn": f["false_alarm_warn"],
                         "why": f["why_exact"]})
        print(name, "done", flush=True)
    (out / "sweep-summary.json").write_text(json.dumps(rows, indent=1))
    for r in rows:
        print(json.dumps(r))


if __name__ == "__main__":
    main()
