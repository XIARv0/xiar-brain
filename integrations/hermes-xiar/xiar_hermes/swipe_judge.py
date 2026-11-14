# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Second-opinion judging for the Swipe Deck: a cloud bot and a bigger local bot give every card a heart or a skull, BLIND.

Which models play the two roles (and whether the owner votes at all) is the owner's choice (Swipe Deck > Judges; <state_dir>/swipe-deck/judges.json). The local bot defaults to the
local model that runs here (nothing leaves the machine); the cloud bot is off until the owner picks a connection. A bot that is on goes through EVERY waiting card, oldest first.
A judge is sent ONLY: the task excerpt, the picked model(s) with their note (trimmed), and for an outcome card the first lines of the agent's answer. It never sees the owner's
vote (the ledger also refuses a bot vote after the owner's), private memory or files. A cloud connection sends that text to its provider: that is the owner's choice.
The provider call is the vetted adapter (integrations/model-connections/call.py: no redirects, TLS for anything not local, credentials by environment reference, nothing logged).
Any failure means no vote (never a made-up one); after two failures on a card the bot is skipped for that card. Standard library only.
"""
from __future__ import annotations

import importlib.util
import json
import os
import pathlib
import re
import tomllib

ROLES = ("cloud", "local")
ROW = re.compile(r"^\|\s*(\d+)\s*\|\s*\[\[([^\]|\\]+)\\?\|([^\]]+)\]\]\s*\|\s*([^|]+?)\s*\|\s*(.*?)\s*\|?\s*$")
NOTE_CHARS = {"suggest": 650, "use": 1500, "outcome": 1100}      # per model, so a small local model is not flooded
_CAT = {}


def _adapter(root):
    spec = importlib.util.spec_from_file_location("xiar_provider_call", pathlib.Path(root) / "integrations/model-connections/call.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_catalogue(root):
    """The 99 models: number, name, family, the 'when to use' line and the note's path (from the Brain's Index)."""
    root = pathlib.Path(root)
    idx = root / "Brain/Shared/mental-models/Index.md"
    if not idx.is_file():
        return []
    stamp = idx.stat().st_mtime
    if _CAT.get(str(root), (None,))[0] == stamp:
        return _CAT[str(root)][1]
    rows = []
    for line in idx.read_text(encoding="utf-8").splitlines():
        m = ROW.match(line)
        if m:
            rows.append({"n": int(m[1]), "name": m[3].strip(), "group": m[4].strip(), "when": m[5].strip(), "rel": m[2].strip()})
    _CAT[str(root)] = (stamp, rows)
    return rows


def read_note(root, n, limit=None):
    """A model's full note, only ever from the Index's own path for that number (no caller-supplied paths)."""
    base = (pathlib.Path(root) / "Brain/Shared/mental-models").resolve()
    for m in load_catalogue(root):
        if m["n"] == int(n):
            p = (base / (m["rel"] + ".md")).resolve()
            if base in p.parents and p.is_file():
                text = p.read_text(encoding="utf-8")
                return text if limit is None else " ".join(text.split())[:limit]
    return ""


def connections(root):
    """The chat connections the owner can choose from: id, and whether it runs here or at a provider."""
    mod = _adapter(root)
    runtime = tomllib.loads((pathlib.Path(root) / "config/llm_runtime.toml").read_text(encoding="utf-8"))
    out = []
    for cid, c in mod.connections(runtime).items():
        if c.get("kind") == "chat":
            local = c.get("type", "local") in ("local", "ollama") and ("127.0.0.1" in str(c.get("base_url", "")) or not c.get("base_url"))
            out.append({"id": cid, "where": "here" if local else "provider"})
    return out


def resolve(root, deck, role):
    """The connection id a role uses now: the owner's choice, or for the local bot with no choice made the first model that runs here."""
    chosen = deck.settings().get(role)
    if chosen == "default":
        return next((c["id"] for c in connections(root) if c["where"] == "here"), None)
    return chosen


def _post(mod, root, c, path, body):
    """The vetted adapter's request. A local connection behind managed auth (config/llm_runtime.toml managed_auth_token_file) gets its bearer the same way the model router does
    (integrations/model-router/managed_auth.py: a private mode-600 file of the same user, loopback only, never printed or logged)."""
    if not c.get("managed_auth_token_file"):
        return mod.request(c, path, body)
    spec = importlib.util.spec_from_file_location("xiar_managed_auth", pathlib.Path(root) / "integrations/model-router/managed_auth.py")
    auth = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(auth)
    import urllib.request
    headers = {"Content-Type": "application/json", **auth.bearer(c)}
    req = urllib.request.Request(mod.api_url(c, path), data=json.dumps(body).encode(), headers=headers)
    with urllib.request.build_opener(mod.NoRedirect).open(req, timeout=float(c.get("timeout_s", 60))) as response:
        raw = response.read(1048577)
    if len(raw) > 1048576:
        raise ValueError("response too large")
    return json.loads(raw)


def _messages(pick, models):
    """models: [{name, when, note}]. What a judge is sent, and nothing else."""
    kind = pick.get("kind", "suggest")
    rows = "\n\n".join("%s (%s)\n%s" % (m["name"], m.get("when", "")[:140], m.get("note", "")) for m in models)
    system = ("You judge how well mental models serve decisions. A mental model is a way of thinking about a decision. Answer with exactly one word: HEART if it was good, SKULL if it was bad.")
    if kind == "use":
        ask = "An agent chose this mental model to make a decision on the task. Was it a good choice for this task?"
    elif kind == "outcome":
        ask = "An agent used this mental model to make a decision on the task. Its answer is shown. Did the model lead to a good decision?"
    else:
        ask = "A system offered these mental models to an agent for the task. Were they good models to offer for this task?"
    user = "%s\n\nTask: %s\n\nMental model%s:\n%s\n" % (ask, pick.get("task") or "(no task text)", "s" if len(models) > 1 else "", rows)
    if kind == "outcome":
        user += "\nThe agent's answer (first lines): %s\n" % (pick.get("answer") or "(none)")
    return [{"role": "system", "content": system}, {"role": "user", "content": user + "\nHEART or SKULL?"}]


def ask(root, connection, pick, models, messages=None):
    """+1 heart, -1 skull, or None (no opinion: unavailable, unreadable, or unsure)."""
    mod = _adapter(root)
    runtime = tomllib.loads((pathlib.Path(root) / "config/llm_runtime.toml").read_text(encoding="utf-8"))
    c = dict(mod.connections(runtime).get(connection) or {})
    if not c or c.get("kind") != "chat":
        return None
    c["timeout_s"] = min(float(c.get("timeout_s", 120)), 180)
    model = c.get("served_name") or c.get("model") or connection
    messages = messages or _messages(pick, models)
    try:
        if c.get("type") == "ollama":
            r = _post(mod, root, c, "/api/chat", {"model": model, "messages": messages, "stream": False, "think": False, "options": {"num_predict": 8, "temperature": 0}})
            text = r["message"]["content"]
        else:
            body = {"model": model, "messages": messages, "max_tokens": 8, "temperature": 0, "stream": False}
            if c.get("thinking", "auto") != "auto":
                body.update(enable_thinking=False, chat_template_kwargs={"enable_thinking": False})
            r = _post(mod, root, c, "/v1/chat/completions", body)
            text = r["choices"][0]["message"]["content"]
    except Exception:
        return None
    word = str(text).strip().lower().lstrip("*#>\"' ")
    if word.startswith("heart"):
        return 1
    if word.startswith("skull"):
        return -1
    return None


def judge_pick(root, deck, pick, roles=ROLES):
    """Every bot that is on and has not judged this card gives its blind opinion. Returns {role: value}. The ledger refuses it if the owner has voted."""
    if hasattr(deck, "judge_models"):          # forecast cards (UpgradePlan 8.13) bring their own models and question
        models = deck.judge_models(root, pick)
    else:
        cat = {m["n"]: m for m in load_catalogue(root)}
        limit = NOTE_CHARS.get(pick.get("kind", "suggest"), 650)
        models = [{"name": cat[n]["name"], "when": cat[n]["when"], "note": read_note(root, n, limit)} if n in cat else {"name": "#%d" % n, "when": "", "note": ""} for n in pick["models"]]
    done = deck.bots_on(pick["id"])
    out = {}
    for role in roles:
        if role in done or role not in deck.enabled_bots():
            continue
        connection = resolve(root, deck, role)
        value = (ask(root, connection, pick, models, deck.judge_messages(pick, models)) if hasattr(deck, "judge_messages") else ask(root, connection, pick, models)) if connection else None
        if value is None:
            deck.note_failure(pick["id"], role)         # twice and this bot is skipped for this card
            continue
        try:
            deck.vote(pick["id"], value, role)
            out[role] = value
        except ValueError:
            pass            # the owner voted first, or it already judged: nothing is written
    return out


def judge_pending(root, deck, limit=None):
    """The bots that are on go through every waiting card, oldest first (limit caps one pass, so a slow model never holds a request open)."""
    results, seen = {}, set()
    for role in ROLES:
        for p in deck.pending_for(role):
            if p["id"] in seen:
                continue
            if limit is not None and len(results) >= limit:
                return results
            seen.add(p["id"])
            results[p["id"]] = judge_pick(root, deck, p)
    return results
