# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Brain HTTP API (bind to localhost; typically run on the machine that has the GPU).

Routes (no route ever returns lesson contents):
  GET  /v1/health                       no auth
  GET  /v1/status        role=query     counts/version only
  POST /v1/signals       role=query     {context, action} -> signals Hermes consumes before acting
  POST /v1/lessons       role=writer    append a lesson to the sealed store (write-only)
  POST /v1/train         role=admin     start an idle-window training job in a separate process
The service holds ONLY the sealed store's public key. It never loads the trainer private key.
"""
from __future__ import annotations

import hmac
import json
import pathlib
import secrets
import subprocess
import sys
import threading
import time

from fastapi import Depends, FastAPI, Header, HTTPException, Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel, Field

from .allowlist import LessonPolicy, LessonRejected
from .brain import Brain, pick_device
from .checkpoints import CheckpointStore
from .config import BrainConfig, load_config
from .sealed_store import SealedWriter

ROLES = ("writer", "query", "admin")


def ensure_tokens(path: pathlib.Path) -> dict:
    if path.exists():
        return json.loads(path.read_text(encoding="utf-8"))
    path.parent.mkdir(parents=True, exist_ok=True)
    toks = {r: secrets.token_urlsafe(32) for r in ROLES}
    path.write_text(json.dumps(toks, indent=2), encoding="utf-8")
    return toks


class SignalReq(BaseModel):
    action: str = Field(min_length=1, max_length=2000)
    context: str = Field(default="", max_length=2000)


class LessonReq(BaseModel):
    kind: str
    action: str
    outcome: str
    context: str = ""
    why: str = ""
    source: str = ""


class BrainHolder:
    def __init__(self, cfg: BrainConfig):
        self.cfg = cfg
        self.ck = CheckpointStore(cfg.path("paths.checkpoints_dir"))
        self.brain: Brain | None = None
        self.version: str | None = None
        self.lock = threading.Lock()
        self._last_check = 0.0
        self.device = pick_device(cfg)

    def get(self) -> tuple[Brain | None, str | None]:
        now = time.time()
        if now - self._last_check >= float(self.cfg.get("service.reload_poll_seconds")) or self.brain is None:
            self._last_check = now
            v = self.ck.manifest().get("promoted")
            if v and v != self.version:
                b = Brain.load(self.cfg, self.ck.path(v), self.device)
                with self.lock:
                    self.brain, self.version = b, v
        return self.brain, self.version


def create_app(cfg: BrainConfig | None = None) -> FastAPI:
    cfg = cfg or load_config()
    tokens = ensure_tokens(cfg.path("paths.tokens_file"))
    policy = LessonPolicy(cfg)
    writer = SealedWriter(cfg.path("paths.sealed_db"), cfg.path("paths.sealed_public_key"))
    holder = BrainHolder(cfg)
    app = FastAPI(title="XIAR NeuralBrain", version="0.1.0", docs_url=None, redoc_url=None, openapi_url=None)
    max_body = int(cfg.get("service.max_body_bytes"))

    def role(required: str):
        def dep(authorization: str = Header(default="")):
            tok = authorization.removeprefix("Bearer ").strip()
            allowed = [required, "admin"]
            if not any(hmac.compare_digest(tok, tokens[r]) for r in allowed if r in tokens):
                raise HTTPException(401, "missing or wrong token for role " + required)
        return Depends(dep)

    @app.middleware("http")
    async def limit(request: Request, call_next):
        if int(request.headers.get("content-length") or 0) > max_body:
            return JSONResponse({"detail": "body too large"}, status_code=413)
        return await call_next(request)

    @app.get("/v1/health")
    def health():
        return {"ok": True}

    @app.get("/v1/status", dependencies=[role("query")])
    def status():
        b, v = holder.get()
        return {"promoted": v, "lessons_sealed": writer.count(), "device": str(holder.device),
                "lessons_seen": b.lessons_seen if b else 0, "windows": b.windows if b else 0,
                "params": b.model.n_params() if b else None,
                "memory_n_keys": [m.n_keys for m in b.model.memories()] if b else None,
                "utilisation": b.utilisation() if b else None, "tokenizer": cfg.get("tokenizer.name")}

    @app.post("/v1/signals", dependencies=[role("query")])
    def signals(req: SignalReq):
        b, v = holder.get()
        if b is None:
            return {"version": None, "signals": [], "note": "newborn: no promoted checkpoint yet"}
        with holder.lock:
            s = b.signals(req.context, req.action)
        keep = s["confidence"] >= float(cfg.get("signals.min_confidence"))
        return {"version": v, "signals": [s] if keep else [], "below_threshold": 0 if keep else 1}

    @app.post("/v1/lessons", status_code=201, dependencies=[role("writer")])
    def lessons(req: LessonReq):
        try:
            clean = policy.validate(req.model_dump())
        except LessonRejected as e:
            raise HTTPException(422, str(e))
        return {"id": writer.append(clean)}

    @app.post("/v1/train", status_code=202, dependencies=[role("admin")])
    def train(force: bool = False):
        args = [sys.executable, "-m", "xiar_brain", "train"] + (["--force"] if force else [])
        subprocess.Popen(args, cwd=str(pathlib.Path(__file__).resolve().parents[1]))
        return {"started": True}

    return app
