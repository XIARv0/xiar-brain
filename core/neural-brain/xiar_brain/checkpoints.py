# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Versioned checkpoints with a manifest; only promoted versions are served. Rollback supported."""
from __future__ import annotations

import json
import pathlib
import time


class CheckpointStore:
    def __init__(self, root: pathlib.Path):
        self.root = root
        self.root.mkdir(parents=True, exist_ok=True)
        self.manifest_path = root / "manifest.json"

    def manifest(self) -> dict:
        if self.manifest_path.exists():
            return json.loads(self.manifest_path.read_text(encoding="utf-8"))
        return {"promoted": None, "history": []}

    def _write(self, m: dict) -> None:
        tmp = self.manifest_path.with_suffix(".tmp")
        tmp.write_text(json.dumps(m, indent=2), encoding="utf-8")
        tmp.replace(self.manifest_path)

    def next_version(self) -> str:
        return f"v{len(self.manifest()['history']) + 1:04d}"

    def path(self, version: str) -> pathlib.Path:
        return self.root / f"{version}.pt"

    def record(self, version: str, promoted: bool, metrics: dict, reason: str) -> None:
        m = self.manifest()
        m["history"].append({"version": version, "promoted": promoted, "reason": reason,
                             "metrics": metrics, "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z")})
        if promoted:
            m["promoted"] = version
        self._write(m)

    def promoted_path(self) -> pathlib.Path | None:
        v = self.manifest()["promoted"]
        return self.path(v) if v else None

    def rollback(self, version: str) -> None:
        m = self.manifest()
        if not self.path(version).exists():
            raise FileNotFoundError(version)
        m["promoted"] = version
        m["history"].append({"version": version, "promoted": True, "reason": "manual rollback",
                             "metrics": {}, "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z")})
        self._write(m)
