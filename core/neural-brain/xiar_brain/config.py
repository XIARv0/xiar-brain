# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Config loading. Every size/path/threshold lives in config/neural_brain.toml (no hard-coded values).

Resolution: $XIAR_BRAIN_CONFIG or <repo>/config/neural_brain.toml. Relative paths resolve against
$XIAR_ROOT or the repo root (parent of the config dir). `XIAR_BRAIN__section__key=value` env vars
override single values (parsed as TOML literals when possible).
"""
from __future__ import annotations

import copy
import os
import pathlib
import tomllib
from dataclasses import dataclass
from typing import Any

PKG_ROOT = pathlib.Path(__file__).resolve().parents[1]          # core/neural-brain
REPO_ROOT_GUESS = PKG_ROOT.parents[1]                           # repo root


def _parse_literal(v: str) -> Any:
    try:
        return tomllib.loads(f"x = {v}")["x"]
    except tomllib.TOMLDecodeError:
        return v


@dataclass
class BrainConfig:
    raw: dict
    root: pathlib.Path
    source: pathlib.Path | None

    def get(self, dotted: str, default: Any = None) -> Any:
        cur: Any = self.raw
        for part in dotted.split("."):
            if not isinstance(cur, dict) or part not in cur:
                return default
            cur = cur[part]
        return cur

    def section(self, name: str) -> dict:
        return dict(self.raw.get(name, {}))

    def path(self, dotted: str) -> pathlib.Path:
        v = self.get(dotted)
        if v is None:
            raise KeyError(f"config path {dotted} missing")
        p = pathlib.Path(os.path.expandvars(os.path.expanduser(str(v))))
        return p if p.is_absolute() else (self.root / p)

    def with_overrides(self, overrides: dict) -> "BrainConfig":
        raw = copy.deepcopy(self.raw)
        for dotted, val in overrides.items():
            cur = raw
            parts = dotted.split(".")
            for p in parts[:-1]:
                cur = cur.setdefault(p, {})
            cur[parts[-1]] = val
        return BrainConfig(raw=raw, root=self.root, source=self.source)


def load_config(path: str | os.PathLike | None = None, root: str | os.PathLike | None = None) -> BrainConfig:
    src = pathlib.Path(path or os.environ.get("XIAR_BRAIN_CONFIG") or REPO_ROOT_GUESS / "config" / "neural_brain.toml")
    raw = tomllib.loads(src.read_text(encoding="utf-8"))
    r = pathlib.Path(root or os.environ.get("XIAR_ROOT") or src.resolve().parents[1])
    for k, v in os.environ.items():
        if k.startswith("XIAR_BRAIN__"):
            parts = k[len("XIAR_BRAIN__"):].lower().split("__")
            cur = raw
            for p in parts[:-1]:
                cur = cur.setdefault(p, {})
            cur[parts[-1]] = _parse_literal(v)
    return BrainConfig(raw=raw, root=r, source=src)
