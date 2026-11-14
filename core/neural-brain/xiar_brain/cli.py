# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""xiar-brain CLI: init | serve | train | status | rollback | fetch-tokenizer"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys

from .config import load_config


def _restrict(path):
    if sys.platform == "win32":
        import getpass
        subprocess.run(["icacls", str(path), "/inheritance:r", "/grant:r", f"{getpass.getuser()}:F"],
                       capture_output=True)


def main(argv=None):
    ap = argparse.ArgumentParser("xiar-brain")
    ap.add_argument("--config")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("init", help="create trainer keypair, API tokens and data dirs")
    s = sub.add_parser("serve"); s.add_argument("--host"); s.add_argument("--port", type=int)
    t = sub.add_parser("train", help="run one idle-window training job"); t.add_argument("--force", action="store_true")
    sub.add_parser("status")
    r = sub.add_parser("rollback"); r.add_argument("version")
    sub.add_parser("fetch-tokenizer")
    a = ap.parse_args(argv)
    cfg = load_config(a.config)
    if sys.stdout is None or sys.stderr is None:  # pythonw (scheduled task, no console): log to a file
        logs = cfg.path("paths.logs_dir"); logs.mkdir(parents=True, exist_ok=True)
        sys.stdout = sys.stderr = open(logs / f"{a.cmd}.log", "a", encoding="utf-8", buffering=1)

    if a.cmd == "init":
        from .sealed_store import _open_db, generate_keys
        from .service import ensure_tokens
        pub, priv = cfg.path("paths.sealed_public_key"), cfg.path("paths.sealed_private_key")
        if not priv.exists():
            generate_keys(pub, priv)
            _restrict(priv)
        _open_db(cfg.path("paths.sealed_db")).close()
        tp = cfg.path("paths.tokens_file")
        ensure_tokens(tp)
        _restrict(tp)
        print(json.dumps({"public_key": str(pub), "private_key": "created (trainer only)", "tokens": str(tp)}))
    elif a.cmd == "serve":
        import uvicorn
        from .service import create_app
        uvicorn.run(create_app(cfg), host=a.host or cfg.get("service.host"), port=a.port or int(cfg.get("service.port")),
                    log_level="warning", access_log=False)
    elif a.cmd == "train":
        from .trainer import run_window
        out = run_window(cfg, force=a.force)
        if not out.get("ran"):
            print(json.dumps(out))
    elif a.cmd == "status":
        from .checkpoints import CheckpointStore
        print(json.dumps(CheckpointStore(cfg.path("paths.checkpoints_dir")).manifest(), indent=2))
    elif a.cmd == "rollback":
        from .checkpoints import CheckpointStore
        CheckpointStore(cfg.path("paths.checkpoints_dir")).rollback(a.version)
        print("promoted", a.version)
    elif a.cmd == "fetch-tokenizer":
        import shutil
        from huggingface_hub import hf_hub_download
        dst = cfg.path("tokenizer.path"); dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(hf_hub_download(cfg.get("tokenizer.hf_repo"), "tokenizer.json"), dst)
        print("tokenizer.json ->", dst)
