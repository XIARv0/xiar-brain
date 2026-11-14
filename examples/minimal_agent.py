# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Minimal agent wired to the XIAR V0 brain system (stdlib only).  Powered by XIAR V0 - https://xiarv0.com

It shows the five hook points the XIAR Hermes adapter uses (integrations/hermes-xiar/xiar_hermes/plugin.py):
  pre_llm_call            brain signals + memory recall -> extra context for the LLM
  pre_tool_call           ask the brain "did this fail / get rejected before?" -> optional approve/block directive
  post_tool_call          tool outcome -> write-only lesson for the brain
  post_approval_response  owner approve/deny -> approval/rejection lesson
  on_session_end          drop per-session state

Services (both bind to 127.0.0.1 by default):
  memory API   xiar-memory-api   http://127.0.0.1:8771   Authorization: Bearer <client token> + X-Business-Id: <slug>
  neural brain xiar_brain        http://127.0.0.1:8770   Authorization: Bearer <query|writer token>

Environment (no secrets are hard-coded; token files are created by the services themselves):
  XIAR_MEMORY_URL         default http://127.0.0.1:8771
  XIAR_MEMORY_TOKEN_FILE  JSON {client name: token} written by `xiar-memory-api init`
  XIAR_MEMORY_CLIENT      client name in that file (default "xiar-owner", see config/memory_api.toml [[clients]])
  XIAR_BUSINESS_ID        business slug every memory call is scoped to (required by the API)
  XIAR_BRAIN_URL          default http://127.0.0.1:8770
  XIAR_BRAIN_TOKENS_FILE  JSON {"writer","query","admin"} written by the brain service on first start
Run:
  python examples/minimal_agent.py "Which vendors supply sensors?"
  python examples/minimal_agent.py --write "Sensor Supply Co" "Delivers evaluation boards for Project Example"
"""
from __future__ import annotations

import json
import os
import pathlib
import sys
import urllib.error
import urllib.request

MEMORY_URL = os.environ.get("XIAR_MEMORY_URL", "http://127.0.0.1:8771").rstrip("/")
BRAIN_URL = os.environ.get("XIAR_BRAIN_URL", "http://127.0.0.1:8770").rstrip("/")
BUSINESS = os.environ.get("XIAR_BUSINESS_ID", "example-business")


def _read_json(path_env: str) -> dict:
    p = os.environ.get(path_env, "")
    try:
        return json.loads(pathlib.Path(os.path.expanduser(p)).read_text(encoding="utf-8")) if p else {}
    except (OSError, ValueError):
        return {}


MEMORY_TOKEN = _read_json("XIAR_MEMORY_TOKEN_FILE").get(os.environ.get("XIAR_MEMORY_CLIENT", "xiar-owner"), "")
BRAIN_TOKENS = _read_json("XIAR_BRAIN_TOKENS_FILE")


def post(url: str, body: dict, token: str, headers: dict | None = None, timeout: float = 3.0) -> tuple[int, dict]:
    """POST JSON; fail open (status 0) when a service is down, like the real adapter."""
    h = {"Content-Type": "application/json", "Authorization": f"Bearer {token}", **(headers or {})}
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers=h, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"{}")
        except ValueError:
            return e.code, {}
    except (urllib.error.URLError, TimeoutError, OSError):
        return 0, {}


# ---------------- clients ----------------
def memory_search(query: str, limit: int = 5) -> list[dict]:
    """POST /v1/semantic/search {query, limit?, ontology?, method?: bm25|embedding|hybrid} -> {hits:[...]}."""
    code, body = post(MEMORY_URL + "/v1/semantic/search", {"query": query, "limit": limit, "ontology": True},
                      MEMORY_TOKEN, {"X-Business-Id": BUSINESS})
    return body.get("hits", []) if code == 200 else []


def memory_insert(name: str, summary: str, details: str = "", entity_types: list[str] | None = None) -> dict:
    """POST /v1/semantic/insert {name, summary, details?, source?, entity_types?} -> 201 {business_id, id, ...}."""
    code, body = post(MEMORY_URL + "/v1/semantic/insert",
                      {"name": name, "summary": summary, "details": details, "source": "minimal_agent",
                       "entity_types": entity_types or []},
                      MEMORY_TOKEN, {"X-Business-Id": BUSINESS})
    return body if code == 201 else {"error": body.get("error") or f"status {code}"}


def brain_signals(context: str, action: str) -> list[dict]:
    """POST /v1/signals {context, action} (role=query) -> {version, signals:[{text, label, confidence, why?}]}."""
    code, body = post(BRAIN_URL + "/v1/signals", {"context": context[:2000], "action": action[:2000]},
                      BRAIN_TOKENS.get("query", ""), timeout=2.0)
    return body.get("signals", []) if code == 200 else []


def brain_lesson(kind: str, outcome: str, action: str, context: str = "", why: str = "") -> bool:
    """POST /v1/lessons (role=writer, write-only; nothing can read lessons back) -> 201 {id}.
    kind: correction|approval|rejection|preference|outcome|mistake ; outcome: ok|fail|rejected|approved|prefer|corrected
    Lessons are behaviour only: secrets, money, emails and business facts are refused (HTTP 422)."""
    code, _ = post(BRAIN_URL + "/v1/lessons",
                   {"kind": kind, "outcome": outcome, "action": action, "context": context, "why": why,
                    "source": "minimal_agent"}, BRAIN_TOKENS.get("writer", ""), timeout=2.0)
    return code == 201


# ---------------- hooks ----------------
def pre_llm_call(session_id: str, user_message: str) -> str:
    parts = []
    sig = brain_signals(f"session {session_id}", f"request: {user_message[:400]}")
    if sig:
        parts.append("[XIAR brain - learned from past experience]\n" +
                     "\n".join(f"- {s['text']} (confidence {s.get('confidence', 0):.2f})" for s in sig))
    hits = memory_search(user_message)
    if hits:
        parts.append(f"[Memory business:{BUSINESS}]\n" + "\n".join(f"- {h.get('name')}: {h.get('summary')}" for h in hits))
    return "\n\n".join(parts)


def pre_tool_call(tool_name: str, args: dict, gate_confidence: float = 0.8) -> dict | None:
    action = f"{tool_name}: {json.dumps(args)[:200]}"
    for s in brain_signals("tool call", action):
        if s.get("label") in ("fail", "rejected") and s.get("confidence", 0) >= gate_confidence:
            return {"action": "approve", "message": f"XIAR brain: {s['text']}"}  # ask the owner first
    return None


def post_tool_call(tool_name: str, args: dict, result: str) -> None:
    failed = result.lstrip().lower().startswith(("error", "traceback", "fatal:"))
    brain_lesson("mistake" if failed else "outcome", "fail" if failed else "ok",
                 f"{tool_name}: {json.dumps(args)[:200]}", why=result[:200] if failed else "")


def post_approval_response(command: str, choice: str, description: str = "") -> None:
    denied = choice.lower() in ("deny", "denied", "reject", "no")
    brain_lesson("rejection" if denied else "approval", "rejected" if denied else "approved",
                 f"terminal: {command}"[:200], why=description[:200])


def on_session_end(session_id: str) -> None:
    pass  # the real adapter drops cached last action/reply for the session here


if __name__ == "__main__":
    args = sys.argv[1:]
    if args and args[0] == "--write":
        if len(args) < 3:
            sys.exit("usage: minimal_agent.py --write NAME SUMMARY [DETAILS]")
        name, summary = args[1], args[2]
        details = " ".join(args[3:]) if len(args) > 3 else ""
        written = memory_insert(name, summary, details)
        if "error" in written:
            print("write failed:", written["error"])
            sys.exit(1)
        print("wrote:", {k: written[k] for k in ("id", "business_id", "name") if k in written})
        if "id" not in written:
            written["name"] = name
        hits = memory_search(name)
        found = any(
            (h.get("id") and h.get("id") == written.get("id")) or h.get("name") == name
            for h in hits
        )
        print("retrieved:", found, "hits:", len(hits))
        if not found:
            sys.exit(2)
    else:
        question = " ".join(args) or "Which vendors supply sensors?"
        print("context for the LLM:\n" + (pre_llm_call("demo", question) or "(no brain/memory context: services down or empty)"))
        print("pre_tool_call directive:", pre_tool_call("terminal", {"command": "ls"}))
        on_session_end("demo")
    print("Powered by XIAR V0 - https://xiarv0.com")
