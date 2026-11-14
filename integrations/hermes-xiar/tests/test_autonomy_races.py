# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""The two plugin races: a stop between approval and execution, and a stop+resume inside one poll."""
import json, pathlib, sys
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from xiar_hermes import autonomy as A


class Agent:
    session_id = 's'
    _xiar_execution_home = None


def run(monkeypatch, stopped_after_begin, mutating):
    calls = []
    state = {'stopped': False}
    monkeypatch.setattr(A, 'state', lambda: dict(stopped=state['stopped'], generation=1))

    def rpc(agent, body):
        calls.append(body['action'])
        if body['action'] == 'begin':
            if stopped_after_begin: state['stopped'] = True   # the owner hits Emergency stop right after approval
            return dict(allowed=True, mutating=mutating, request_id='r1')
        return {}
    monkeypatch.setattr(A, 'rpc', rpc)
    ran = []
    out = A.guarded(Agent(), 't', {'a': 1}, lambda a: ran.append(1) or json.dumps({'ok': True}))
    return out, ran, calls


def test_stop_after_approval_blocks_a_mutating_tool(monkeypatch):
    out, ran, calls = run(monkeypatch, True, True)
    assert not ran and 'was not run' in json.loads(out)['error']
    assert calls[-1] == 'finish'   # the ticket is closed as failed, not left executing


def test_stop_after_approval_does_not_block_a_read_only_tool(monkeypatch):
    out, ran, _ = run(monkeypatch, True, False)
    assert ran and json.loads(out) == {'ok': True}


def test_no_stop_runs_normally(monkeypatch):
    out, ran, _ = run(monkeypatch, False, True)
    assert ran and json.loads(out) == {'ok': True}


def test_interrupt_rules():
    n = A.needs_interrupt
    assert not n(dict(stopped=False), 4, 4, False)        # nothing changed
    assert n(dict(stopped=True), 5, 4, False)             # a new stop
    assert not n(dict(stopped=True), 5, 5, True)          # the same stop, already handled
    assert not n(dict(stopped=False), 6, 5, True)         # the resume after a stop we already saw
    assert n(dict(stopped=False), 6, 4, False)            # stop+resume inside one poll: missed stop
    assert n(dict(stopped=True), -1, 4, False)            # authority unavailable counts as stopped
