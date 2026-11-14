# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
import json
import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from xiar_hermes import plugin  # noqa: E402
from xiar_hermes.config import AdapterConfig, load  # noqa: E402

REPO = pathlib.Path(__file__).resolve().parents[3]


class FakeBrain:
    def __init__(self, sig=None):
        self.sig, self.lessons, self.queries = sig or [], [], []

    def signals(self, context, action):
        self.queries.append(action)
        return self.sig

    def lesson(self, l):
        self.lessons.append(l)
        return True


class FakeMemory:
    def search(self, business_id, query, limit, read_agent=None, read_user=None):   # the plugin scopes reads to the calling agent and user
        if not business_id:
            return {"error": "business_id is required"}
        return {"business_id": business_id, "hits": [{"name": "Sensor Supply Co", "summary": "Supplies sensors"}]}


class Ctx:
    def __init__(self):
        self.hooks, self.tools = {}, {}

    def register_hook(self, name, cb):
        self.hooks[name] = cb

    def register_tool(self, name, toolset, schema, handler, **kw):
        self.tools[name] = (schema, handler)


@pytest.fixture
def cfg(tmp_path):
    c = load(str(REPO / "config" / "hermes_adapter.toml"))
    mm = tmp_path / "mm"; (mm / "General").mkdir(parents=True)
    (mm / "Index.md").write_text(
        "| # | Model | Group | When to use |\n|---|---|---|---|\n"
        "| 7 | [[General/7 Inversion\\|Inversion]] | General | Risk management and safety when the cost of failure is high |\n"
        "| 9 | [[General/9 Occam's Razor\\|Occam's Razor]] | General | Choosing between two theories that explain the same data |\n",
        encoding="utf-8")
    (mm / "General" / "7 Inversion.md").write_text("# Inversion\nThink backwards.", encoding="utf-8")
    raw = json.loads(json.dumps(c.raw))
    raw["mental_models"]["dir"] = str(mm)
    raw["capture"]["ok_sample_rate"] = 0.0
    for job in ("thinking_models", "correction_detection", "retrieval_strategy"):
        raw["laya"]["jobs"][job] = False
    raw["routing"]["state_dir"] = str(tmp_path / "state")
    return AdapterConfig(raw, c.root)


class NoLaya:
    url, http = "http://laya.test", type("H", (), {"token": "t"})()

    def decide(self, text, questions):
        return None

    def healthy(self):
        return False


def make(cfg, sig=None):
    return plugin.XiarAdapter(cfg, brain=FakeBrain(sig), memory=FakeMemory(), laya=NoLaya(), background=False)


def test_registers_hooks_and_tools(cfg):
    ctx = Ctx(); plugin.register(ctx, make(cfg))
    assert set(ctx.hooks) == {"post_llm_call", "pre_llm_call", "pre_tool_call", "post_tool_call", "post_approval_response", "on_session_end"}
    assert set(ctx.tools) == {"xiar_memory_search", "xiar_mental_model", "xiar_record_lesson",            # memory and lessons
                              "xiar_dispatch", "spawn_subagent", "cancel_subagent", "list_running", "queue_status",
                              "inspect_agent_logs", "inspect_code_tree", "inspect_config", "inspect_hardware", "inspect_logs",
                              "inspect_memory_catalogue", "inspect_processes", "inspect_sessions",              # self-inspection
                              "ui_open_view", "ui_update_view", "ui_close_view", "ui_list_views",                # views
                              "xiar_drive_start", "xiar_drive_action", "xiar_drive_status", "xiar_drive_end",    # desktop driving
                              "xiar_screen_watch", "xiar_emergency_stop", "godseye_history",
                              "xiar_forecast", "xiar_forecast_outcome"}                                          # number forecasts (8.13)


def test_gate_escalates_on_failed_before(cfg):
    a = make(cfg, [{"label": "fail", "text": "failed before", "confidence": 0.93, "why": "it broke the boot"}])
    d = a.pre_tool_call("terminal", {"command": "update-grub"}, task_id="t1")
    assert d["action"] == "approve" and "failed before" in d["message"] and "broke the boot" in d["message"]
    weak = make(cfg, [{"label": "fail", "text": "failed before", "confidence": 0.5}])
    assert weak.pre_tool_call("terminal", {"command": "ls"}, task_id="t1") is None


def test_capture_failure_success_approval_correction(cfg):
    a = make(cfg)
    a.post_tool_call("terminal", {"command": "make"}, json.dumps({"exit_code": 2, "output": "x"}), "t")
    a.post_tool_call("terminal", {"command": "ls"}, json.dumps({"exit_code": 0}), "t")
    a.post_approval_response(command="rm -rf build", choice="deny", surface="cli")
    a.pre_tool_call("messages", {"action": "send the summary"}, task_id="s1", session_id="s1")
    a.pre_llm_call(session_id="s1", user_message="no, always draft it first", platform="cli")
    kinds = [(l["kind"], l["outcome"]) for l in a.brain.lessons]
    assert kinds == [("mistake", "fail"), ("rejection", "rejected"), ("correction", "corrected")]
    assert a.brain.lessons[2]["action"] == "messages: send the summary"


def test_pre_llm_call_injects_signals_and_models(cfg):
    a = make(cfg, [{"label": "prefer", "text": "owner prefers", "why": "short bullet points", "confidence": 0.8}])
    out = a.pre_llm_call(session_id="s", user_message="what is the risk of failure if we deploy", platform="cli")
    assert "owner prefers: short bullet points" in out["context"] and "Inversion" in out["context"]


def test_mental_model_tool_and_memory_scoping(cfg):
    a = make(cfg)
    assert "Think backwards" in json.loads(a.tool_mental_model({"name": "Inversion"}))["note"]
    assert json.loads(a.tool_memory_search({"query": "vendors"}))["error"] == "business_id is required"
    assert json.loads(a.tool_memory_search({"query": "vendors", "business_id": "tenant-a"}))["business_id"] == "tenant-a"


def test_brain_client_has_no_lesson_read_path():
    from xiar_hermes.clients import BrainClient
    public = {n for n in dir(BrainClient) if not n.startswith("_")}
    assert public == {"signals", "lesson"}


def test_smart_and_timeout_approvals_not_learned(cfg):
    a = make(cfg)
    for ch in ("timeout", "smart_approve", "smart_deny", ""):
        a.post_approval_response(command="x", choice=ch, surface="cli")
    a.post_approval_response(command="git push", choice="always", surface="cli")
    assert [(l["kind"], l["outcome"]) for l in a.brain.lessons] == [("approval", "approved")]
