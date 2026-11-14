# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Adapter config: $XIAR_ADAPTER_CONFIG or <XIAR_ROOT>/config/hermes_adapter.toml. Paths relative to XIAR_ROOT."""
from __future__ import annotations

import os
import pathlib
import tomllib

HERE = pathlib.Path(__file__).resolve()


def xiar_root() -> pathlib.Path:
    if os.environ.get("XIAR_ROOT"):
        return pathlib.Path(os.environ["XIAR_ROOT"])
    return HERE.parents[3]  # <root>/integrations/hermes-xiar/xiar_hermes/config.py


class AdapterConfig:
    def __init__(self, raw: dict, root: pathlib.Path):
        self.raw, self.root = raw, root

    def get(self, dotted, default=None):
        cur = self.raw
        for p in dotted.split("."):
            if not isinstance(cur, dict) or p not in cur:
                return default
            cur = cur[p]
        return cur

    def path(self, dotted) -> pathlib.Path:
        p = pathlib.Path(os.path.expandvars(os.path.expanduser(str(self.get(dotted)))))
        return p if p.is_absolute() else self.root / p


def load(path: str | None = None, raw_override: dict | None = None) -> AdapterConfig:
    root = xiar_root()
    if raw_override is not None:
        return AdapterConfig(raw_override, root)
    if path is None and os.environ.get("XIAR_RUNTIME_MANIFEST"):
        from hermes_constants import get_hermes_home
        from .runtime_profiles import RuntimeProfiles
        return RuntimeProfiles().scoped_config(get_hermes_home())
    if path is None and (root / "config/agent_profiles.toml").is_file():
        try:
            from hermes_constants import get_hermes_home, get_process_hermes_home
        except ImportError:
            pass  # Standalone tools do not import/install native Hermes.
        else:
            from .profiles import Profiles
            scoped = Profiles(root).scoped_config(get_hermes_home())
            if scoped is not None:
                return scoped
            if get_hermes_home().resolve() != get_process_hermes_home().resolve():
                raise ValueError("Unbound native profile cannot inherit Xiar identity or credentials")
    src = pathlib.Path(path or os.environ.get("XIAR_ADAPTER_CONFIG") or root / "config" / "hermes_adapter.toml")
    return AdapterConfig(tomllib.loads(src.read_text(encoding="utf-8")), root)
