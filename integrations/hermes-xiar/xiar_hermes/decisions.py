# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Laya decisions: validated probabilities and metadata-only audit (never lesson text)."""
from __future__ import annotations
import json
import math
import os
import pathlib
import threading
import time

_LOCK = threading.Lock()

def probability(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or not 0 <= value <= 1:
        raise ValueError("invalid probability")
    return float(value)

def choice(answer, count):
    values = answer.get("probabilities", [])
    if len(values) != count:
        raise ValueError("invalid choice shape")
    values = [probability(p) for p in values]
    total = sum(values)
    if not total:
        raise ValueError("empty choice")
    return [p / total for p in values]

def score(answer, levels):
    p = answer.get("level_probabilities", [])
    if len(p) != levels:
        raise ValueError("invalid score shape")
    p = [probability(v) for v in p]
    if not sum(p):
        raise ValueError("empty score")
    return sum(i * v for i, v in enumerate(p)) / sum(p) / (levels - 1)

class Audit:
    def __init__(self, cfg):
        self.path = pathlib.Path(os.path.expanduser(str(cfg.get("routing.state_dir", "~/.local/state/xiar")))) / "laya-jobs.jsonl"
    def write(self, job, reason, **values):
        record = dict(time=time.time(), job=job, reason=reason, **values)
        with _LOCK:
            self.path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            fd = os.open(self.path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
            try:
                os.write(fd, (json.dumps(record, allow_nan=False) + "\n").encode())
            finally:
                os.close(fd)