# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""The 99 human-genius thinking models for Hermes, read from the Brain mirror (never copied into Hermes)."""
from __future__ import annotations

import math
import pathlib
import re
from collections import Counter
from dataclasses import dataclass

ROW = re.compile(r"^\|\s*(\d+)\s*\|\s*\[\[([^\]|\\]+)\\?\|([^\]]+)\]\]\s*\|\s*([^|]+?)\s*\|\s*(.*?)\s*\|\s*$")
WORD = re.compile(r"[a-z0-9]+")
STOP = set("a an the and or of to in on for with when is are be this that it as by at from you your we our use using "
           "into than then can will if not do does what which how why".split())


@dataclass
class Model:
    num: int
    name: str
    group: str
    when: str
    rel: str


def _toks(s: str) -> list[str]:
    return [w for w in WORD.findall(s.lower()) if w not in STOP and len(w) > 2]


class MentalModels:
    def __init__(self, root: pathlib.Path):
        self.root = root
        self.models: list[Model] = []
        idx = root / "Index.md"
        if idx.exists():
            for line in idx.read_text(encoding="utf-8").splitlines():
                m = ROW.match(line)
                if m:
                    self.models.append(Model(int(m[1]), m[3].strip(), m[4].strip(), m[5].strip(), m[2].strip()))
        docs = [Counter(_toks(f"{x.name} {x.name} {x.when}")) for x in self.models]
        df = Counter(t for d in docs for t in d)
        n = max(1, len(docs))
        self._idf = {t: math.log((n + 1) / (c + 1)) + 1 for t, c in df.items()}
        self._docs = docs

    def __len__(self):
        return len(self.models)

    def suggest(self, text: str, k: int) -> list[Model]:
        q = _toks(text)
        scored = []
        for m, d in zip(self.models, self._docs):
            s = sum(d[t] * self._idf.get(t, 0) for t in q)
            if s > 0:
                scored.append((s, m))
        scored.sort(key=lambda x: (-x[0], x[1].num))
        return [m for _, m in scored[:k]]

    def get(self, name_or_num: str) -> str | None:
        key = str(name_or_num).strip().lower()
        for m in self.models:
            if key in (str(m.num), m.name.lower()):
                p = self.root / f"{m.rel}.md"
                return p.read_text(encoding="utf-8") if p.exists() else f"{m.name} ({m.group}): {m.when}"
        return None
