# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Configurable XIAR paths for the autonomy service. Powered by XIAR V0 - https://xiarv0.com

Production defaults stay under /etc/xiar, /var/lib, /run and /opt/xiar and require root
ownership. Local development uses XIAR_AUTONOMY_DEV=1 together with XIAR_AUTONOMY_CONFIG
pointing at a file that is *not* /etc/xiar/autonomy.toml (for example config/autonomy.dev.toml).
"""
from __future__ import annotations

import os
import pwd
import re
from pathlib import Path

PRODUCTION_POLICY = Path('/etc/xiar/autonomy.toml')
PRODUCTION_STATE = Path('/etc/xiar/autonomy-state.json')
PRODUCTION_CONSENT = Path('/etc/xiar/screen-consent.json')
PRODUCTION_CONTROL_TOML = Path('/etc/xiar/agent-control.toml')
PRODUCTION_AUTH_ROOT = Path('/etc/xiar/agent-control-auth')
PRODUCTION_CONTROL_DIR = Path('/var/lib/xiar-agent-control')
PRODUCTION_GRANTS = Path('/etc/xiar/memory-grants.json')
PRODUCTION_SCHEDULER_SHA = Path('/etc/xiar/scheduler-memory.sha256')


def _env_path(name: str) -> Path | None:
    raw = os.environ.get(name, '').strip()
    if not raw:
        return None
    return Path(os.path.expanduser(raw))


def project_root() -> Path:
    if p := _env_path('XIAR_ROOT'):
        return p
    # integrations/agent-control/paths.py -> repository root
    return Path(__file__).resolve().parents[2]


def dev_mode() -> bool:
    """Explicit local-dev only; ignored when the policy path is still the production file."""
    flag = os.environ.get('XIAR_AUTONOMY_DEV', '').strip().lower()
    if flag not in ('1', 'true', 'yes'):
        return False
    return policy_path().resolve() != PRODUCTION_POLICY.resolve()


def policy_path() -> Path:
    if p := _env_path('XIAR_AUTONOMY_CONFIG'):
        return p
    return PRODUCTION_POLICY


def state_path() -> Path:
    if p := _env_path('XIAR_AUTONOMY_STATE'):
        return p
    try:
        import tomllib
        cfg = tomllib.loads(policy_path().read_text(encoding='utf-8'))
        raw = (cfg.get('control') or {}).get('state_file')
        if raw:
            p = Path(os.path.expanduser(str(raw)))
            return p if p.is_absolute() else project_root() / p
    except (OSError, ValueError, TypeError):
        pass
    return PRODUCTION_STATE


def screen_consent_path() -> Path:
    if p := _env_path('XIAR_SCREEN_CONSENT'):
        return p
    return PRODUCTION_CONSENT


def agent_control_config() -> Path:
    if p := _env_path('XIAR_AGENT_CONTROL_CONFIG'):
        return p
    return PRODUCTION_CONTROL_TOML


def agent_control_auth() -> Path:
    if p := _env_path('XIAR_AGENT_CONTROL_AUTH'):
        return p
    return PRODUCTION_AUTH_ROOT


def control_dir() -> Path:
    if p := _env_path('XIAR_AGENT_CONTROL_DIR'):
        return p
    if dev_mode():
        return project_root() / 'data' / 'dev' / 'agent-control'
    return PRODUCTION_CONTROL_DIR


def grants_path() -> Path:
    if p := _env_path('XIAR_MEMORY_GRANTS'):
        return p
    dev = project_root() / 'data' / 'agents' / 'memory-grants.json'
    if dev_mode() or (dev.is_file() and not PRODUCTION_GRANTS.is_file()):
        return dev
    return PRODUCTION_GRANTS


def scheduler_memory_sha() -> Path:
    if p := _env_path('XIAR_SCHEDULER_MEMORY_SHA256'):
        return p
    return PRODUCTION_SCHEDULER_SHA


def owner():
    """pwd entry for the desktop/owner account. XIAR_OWNER_USER, else 'xiar' if it exists, else the current user."""
    name = os.environ.get('XIAR_OWNER_USER', '').strip()
    if name:
        return pwd.getpwnam(name)
    try:
        return pwd.getpwnam('xiar')
    except KeyError:
        return pwd.getpwuid(os.getuid())


def hermes_native() -> Path:
    """Optional Hermes checkout. Missing is fine; hermes features stay off."""
    if p := _env_path('XIAR_HERMES_NATIVE'):
        return p
    return Path(owner().pw_dir) / '.hermes' / 'hermes-agent'


def overflow_uid() -> int:
    try:
        return int(Path('/proc/sys/kernel/overflowuid').read_text().strip())
    except (OSError, ValueError):
        return 65534


def trusted_uids() -> tuple[int, ...]:
    if dev_mode():
        return (os.getuid(),)
    return (0, overflow_uid())


def write_uids(gid: int | None = None) -> tuple[int, int]:
    """UID/GID used when writing root-owned state. Dev (or a non-root process) writes as the current user."""
    if dev_mode():
        return os.getuid(), (os.getgid() if gid is None else gid)
    if os.geteuid() != 0:
        raise PermissionError('Autonomy state must be written as root, or set XIAR_AUTONOMY_DEV=1 with a non-/etc policy')
    return 0, (0 if gid is None else gid)


def desktop_uid(cfg: dict | None = None) -> int:
    """Session UID for optional desktop-drive helpers. 0 / omitted in config means detect."""
    if cfg:
        raw = cfg.get('desktop_uid')
        if raw not in (None, 0, '0', ''):
            return int(raw)
    env = os.environ.get('XIAR_DESKTOP_UID', '').strip()
    if env:
        return int(env)
    runtime = os.environ.get('XDG_RUNTIME_DIR', '')
    m = re.fullmatch(r'/run/user/(\d+)', runtime)
    if m:
        return int(m.group(1))
    return os.getuid()


def desktop_runtime_dir(cfg: dict | None = None) -> Path:
    if cfg:
        raw = (cfg.get('runtime_dir') or '').strip()
        if raw:
            return Path(os.path.expanduser(raw))
    env = os.environ.get('XDG_RUNTIME_DIR', '').strip()
    if env:
        return Path(env)
    return Path(f'/run/user/{desktop_uid(cfg)}')


def timezone_name(cfg: dict | None = None) -> str:
    """IANA name. Empty / 'system' / omitted uses TZ or UTC (never a machine-specific default)."""
    if cfg:
        raw = str(cfg.get('timezone') or '').strip()
        if raw and raw.lower() not in ('system', 'local'):
            return raw
    tz = os.environ.get('TZ', '').strip()
    if tz:
        return tz
    try:
        key = Path('/etc/localtime').resolve().as_posix()
        # .../zoneinfo/Area/City
        if '/zoneinfo/' in key:
            return key.split('/zoneinfo/', 1)[1]
    except OSError:
        pass
    return 'UTC'


def resolve_path(raw, root: Path | None = None) -> Path:
    p = Path(os.path.expanduser(str(raw)))
    if p.is_absolute():
        return p
    return (root or project_root()) / p
