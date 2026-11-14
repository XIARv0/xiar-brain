# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""HTTP clients (stdlib only, fail-open: Hermes keeps working when XIAR services are down)."""
from __future__ import annotations

import json
import logging
import pathlib
import urllib.error
import urllib.request

log = logging.getLogger("xiar_hermes")


def _read_secret(path: pathlib.Path, key: str | None = None) -> str:
    try:
        txt = path.read_text(encoding="utf-8").strip()
    except OSError:
        return ""
    if key:
        try:
            return json.loads(txt).get(key, "")
        except ValueError:
            return ""
    return txt


class Http:
    def __init__(self, base: str, token: str, timeout: float, extra_headers: dict | None = None):
        self.base, self.token, self.timeout = base.rstrip("/"), token, timeout
        self.extra = extra_headers or {}

    def post(self, path: str, body: dict, headers: dict | None = None) -> tuple[int, dict]:
        data = json.dumps(body).encode()
        h = {"Content-Type": "application/json", "Authorization": f"Bearer {self.token}", **self.extra, **(headers or {})}
        req = urllib.request.Request(self.base + path, data=data, headers=h, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as r:
                return r.status, json.loads(r.read() or b"{}")
        except urllib.error.HTTPError as e:
            try:
                return e.code, json.loads(e.read() or b"{}")
            except ValueError:
                return e.code, {}
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            log.debug("xiar service %s unreachable: %s", self.base, e)
            return 0, {}


class BrainClient:
    """Query (signals) and write-only lesson submission. Has no way to read lessons back."""

    def __init__(self, cfg):
        tf = cfg.path("brain.tokens_file")
        t = float(cfg.get("brain.timeout_s", 2.0))
        self.enabled = bool(cfg.get("brain.enabled", True)) and cfg.get("identity.agent_id", "xiar") == "xiar"
        try:
            from hermes_constants import get_hermes_home
            self.emergency_home=str(get_hermes_home())
        except ImportError:self.emergency_home=None
        self.q = Http(cfg.get("brain.url"), _read_secret(tf, "query") if self.enabled else "", t)
        self.w = Http(cfg.get("brain.url"), _read_secret(tf, "writer") if self.enabled else "", t)

    def signals(self, context: str, action: str) -> list[dict]:
        if not self.enabled:
            return []
        code, body = self.q.post("/v1/signals", {"context": context[:2000], "action": action[:2000]})
        return body.get("signals", []) if code == 200 else []

    def lesson(self, lesson: dict) -> bool:
        from .autonomy import readonly
        if readonly(self.emergency_home):return False
        if not self.enabled:
            return False
        code, _ = self.w.post("/v1/lessons", lesson)
        return code == 201


class MemoryClient:
    """MIRIX tenant-scoped semantic memory (xiar-memory-api). business_id is mandatory on every call."""

    def __init__(self, cfg):
        self.enabled = bool(cfg.get("memory.enabled", True))
        self.http = Http(cfg.get("memory.url"), _read_secret(cfg.path("memory.token_file")) if self.enabled else "", float(cfg.get("memory.timeout_s", 3.0)),
            {"X-Agent-Id":str(cfg.get("identity.agent_id", "xiar")), "X-End-User-Id":str(cfg.get("identity.end_user_id", "owner"))})
        self.ontology = bool(cfg.get("memory.ontology", True))

    def search(self, business_id: str, query: str, limit: int, ontology: bool | None = None, read_agent="", read_user="") -> dict:
        if not self.enabled:
            return {"error": "Rust memory disabled for this Agent"}
        if not business_id:
            return {"error": "business_id is required"}
        headers = {"X-Business-Id": business_id}
        if read_agent: headers["X-Read-Agent-Id"] = read_agent
        if read_user: headers["X-Read-User-Id"] = read_user
        code, body = self.http.post("/v1/semantic/search", {"query": query, "limit": limit, "ontology": self.ontology if ontology is None else ontology}, headers=headers)
        if code != 200:
            return {"error": body.get("error") or f"memory api status {code}"}
        return body

    def insert(self, business_id: str, name: str, summary: str, details: str = "", source: str = "",
               entity_types: list[str] | None = None) -> dict:
        """MIRIX Semantic insert into ONE business. The API checks the token may touch that business (tenant backstop)."""
        if not self.enabled:
            return {"error": "Rust memory disabled for this Agent"}
        if not business_id:
            return {"error": "business_id is required"}
        code, body = self.http.post("/v1/semantic/insert", {"name": name, "summary": summary, "details": details,
                                                            "source": source, "entity_types": entity_types or []},
                                    headers={"X-Business-Id": business_id})
        if code != 201:
            return {"error": body.get("error") or f"memory api status {code}"}
        return body

    def populate(self,business_id,text,session_id=""):
        if not self.enabled:
            return {"error": "Rust memory disabled for this Agent"}
        import hashlib
        key=hashlib.sha256((session_id+"\0"+text).encode()).hexdigest()
        code,body=self.http.post("/v1/jobs/enqueue",dict(job="meta",text=text,source="hermes:population",idempotency_key=key),headers={"X-Business-Id":business_id})
        return body if code==202 else {"error":"population enqueue refused"}


class LayaClient:
    """The resident Laya service (xiar-laya.service, `xiar-laya-runner serve`). Fail-open: no answer -> None."""

    def __init__(self, cfg):
        self.enabled = bool(cfg.get("laya.enabled", True))
        self.url = str(cfg.get("laya.url", "http://127.0.0.1:8772")).rstrip("/")
        self.http = Http(self.url, _read_secret(cfg.path("laya.token_file")), float(cfg.get("laya.timeout_s", 5.0)))

    def decide(self, text: str, questions: list[dict]) -> list[dict] | None:
        if not self.enabled:
            return None
        code, body = self.http.post("/v1/decide", {"text": text, "questions": questions})
        answers = body.get("answers") if code == 200 else None
        return answers if isinstance(answers, list) and len(answers) == len(questions) else None

    def healthy(self) -> bool:
        try:
            with urllib.request.urlopen(urllib.request.Request(self.url + "/health",headers={"Authorization":"Bearer "+self.http.token}), timeout=2) as r:
                return r.status == 200 and json.loads(r.read()).get("ok") is True
        except (urllib.error.URLError, TimeoutError, OSError, ValueError):
            return False
