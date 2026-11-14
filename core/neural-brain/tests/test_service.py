# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
import json
import pytest
from fastapi.testclient import TestClient
from xiar_brain.service import create_app
from xiar_brain.trainer import run_window

LESSON = {"kind": "rejection", "action": "messages: post the orb update publicly", "outcome": "rejected",
          "why": "SEALED-WHY-TEXT", "context": "session cli"}


@pytest.fixture
def client(keys):
    app = create_app(keys)
    toks = json.loads(keys.path("paths.tokens_file").read_text())
    return TestClient(app), toks, keys


def h(t):
    return {"Authorization": f"Bearer {t}"}


def test_no_route_returns_lessons(client):
    c, toks, cfg = client
    paths = sorted(r.path for r in c.app.routes)
    assert paths == ["/v1/health", "/v1/lessons", "/v1/signals", "/v1/status", "/v1/train"]
    lesson_route = [r for r in c.app.routes if r.path == "/v1/lessons"][0]
    assert lesson_route.methods == {"POST"}
    r = c.post("/v1/lessons", json=LESSON, headers=h(toks["writer"]))
    assert r.status_code == 201 and "SEALED-WHY-TEXT" not in r.text
    for p in ("/v1/status",):
        assert "SEALED-WHY-TEXT" not in c.get(p, headers=h(toks["query"])).text
    assert c.get("/v1/lessons", headers=h(toks["admin"])).status_code == 405


def test_roles_enforced(client):
    c, toks, _ = client
    assert c.post("/v1/lessons", json=LESSON).status_code == 401
    assert c.post("/v1/lessons", json=LESSON, headers=h(toks["query"])).status_code == 401
    assert c.post("/v1/signals", json={"action": "x"}, headers=h(toks["writer"])).status_code == 401
    assert c.post("/v1/train", headers=h(toks["query"])).status_code == 401


def test_policy_enforced_at_api(client):
    c, toks, _ = client
    bad = dict(LESSON, why="api_key: abc")
    r = c.post("/v1/lessons", json=bad, headers=h(toks["writer"]))
    assert r.status_code == 422 and "abc" not in r.text


def test_newborn_then_trained_signals(client):
    c, toks, cfg = client
    r = c.post("/v1/signals", json={"action": LESSON["action"], "context": "session cli"}, headers=h(toks["query"]))
    assert r.json()["signals"] == [] and r.json()["version"] is None
    for i in range(6):
        c.post("/v1/lessons", json=dict(LESSON, action=LESSON["action"] + f" {i}"), headers=h(toks["writer"]))
    out = run_window(cfg.with_overrides({"train.epochs_per_window": 2}), force=True, log=lambda *_: None)
    assert out["ran"] and out["promoted"]
    c.app  # service reloads the promoted checkpoint
    cfg2 = cfg.with_overrides({"service.reload_poll_seconds": 0})
    c2 = TestClient(create_app(cfg2))
    r = c2.post("/v1/signals", json={"action": LESSON["action"] + " 0", "context": "session cli"}, headers=h(toks["query"]))
    assert r.json()["version"] == "v0001"
