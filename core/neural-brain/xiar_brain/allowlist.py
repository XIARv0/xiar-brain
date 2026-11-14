# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Lesson validation: behaviour/lessons only. Secrets, money, contact details and business facts are refused."""
from __future__ import annotations

import re

from .config import BrainConfig

FIELDS = ("kind", "action", "context", "outcome", "why", "source")


class LessonRejected(ValueError):
    pass


class LessonPolicy:
    def __init__(self, cfg: BrainConfig):
        self.kinds = set(cfg.get("lessons.kinds"))
        self.outcomes = set(cfg.get("lessons.outcomes"))
        self.max_chars = int(cfg.get("lessons.max_field_chars"))
        self.deny = [re.compile(p) for p in cfg.get("lessons.deny_patterns", [])]

    def validate(self, lesson: dict) -> dict:
        extra = set(lesson) - set(FIELDS) - {"ts_ms", "id"}
        if extra:
            raise LessonRejected(f"fields not allowed in lessons: {sorted(extra)}")
        if lesson.get("kind") not in self.kinds:
            raise LessonRejected("kind not in lessons.kinds")
        if lesson.get("outcome") not in self.outcomes:
            raise LessonRejected("outcome not in lessons.outcomes")
        if not str(lesson.get("action", "")).strip():
            raise LessonRejected("action is required")
        out = {}
        for f in FIELDS:
            v = str(lesson.get(f, "") or "")
            if len(v) > self.max_chars:
                raise LessonRejected(f"{f} longer than lessons.max_field_chars")
            for i, rx in enumerate(self.deny):
                if rx.search(v):
                    raise LessonRejected(f"{f} matches deny pattern #{i} (secrets/money/contact/business data are not lessons)")
            out[f] = v
        for k in ("ts_ms", "id"):
            if k in lesson:
                out[k] = lesson[k]
        return out
