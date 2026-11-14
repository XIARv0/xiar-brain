# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Coordinator-level proof of the exact-undo and orphan-reconciliation paths (autonomy.py + autonomy_effects.py), without the memory API, snapper, a scheduler or
the network. Needs Linux, root and the patched external SQLite, exactly like the broker:
  sudo env LD_PRELOAD=/opt/xiar/sqlite/3.54.1/libsqlite3.so.0 PYTHONPATH=<agent-control>:<hermes-xiar> python3 tests/test_autonomy_effects_flow.py"""
import json, os, pathlib, shutil, sys, tempfile, time, tomllib, types, unittest
HERE = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
stub = types.ModuleType('autonomy_drive')
class Drive:
    def __init__(self, a): pass
    def state(self): return {}
    def stop(self, reason): pass
    def request(self, who, body): return {}
stub.Drive = Drive; sys.modules['autonomy_drive'] = stub
import runtime, autonomy, scheduler_store

POLICY = tomllib.loads(pathlib.Path(os.environ.get('XIAR_POLICY') or HERE.parents[1] / 'config/autonomy.toml').read_text(encoding='utf-8'))

class Scheduler:
    def __init__(self, store): self.store = store; self.lifecycle = None

class Flow(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix='af-test-'))
        self.root = self.tmp / 'root'; (self.root / 'config').mkdir(parents=True)
        (self.root / 'config/hermes_adapter.toml').write_text('[identity]\nagent_id = "xiar"\nend_user_id = "owner"\n')
        (self.root / 'config/llm_runtime.toml').write_text('[chat]\nport = 1111\n')
        (self.root / 'config/settings_panel.toml').write_text('[ollama]\nservers = ["http://a"]\n')
        for p in (self.root, self.root / 'config'): os.chmod(p, 0o755)
        runtime.ROOT = self.root
        self.store = scheduler_store.Store(self.tmp / 'store')
        policy = json.loads(json.dumps(POLICY)); self.policy = policy
        autonomy.Autonomy.cfg = lambda s: policy
        autonomy.Autonomy.publish = lambda s, v: None
        autonomy.Autonomy.decision = lambda s, who, tool, args, surface='hermes', approved=False: dict(allowed=True, tier='logged_undoable', mutating=True, forbidden=False, audit_seq=1)
        autonomy.Autonomy.snapshot = lambda s, request: 7
        autonomy.memory.request = lambda path, body, headers: (200, {'audit_seq': 1})
        autonomy.memory.service_key = lambda base: 'k'
        self.a = autonomy.Autonomy(Scheduler(self.store))

    def tearDown(self): shutil.rmtree(self.tmp, ignore_errors=True)

    def gate(self, tool, args, before_state=None):
        return self.a.admin('autonomy_native_gate', dict(tool=tool, args=args, before_state=before_state), None)

    def finish(self, ticket, success=True):
        return self.a.admin('autonomy_native_finish', dict(request_id=ticket['request_id'], success=success), None)

    def row(self, request):
        with self.store.db() as db: return db.execute('SELECT * FROM autonomy_requests WHERE id=?', (request,)).fetchone()

    def test_file_backed_action_is_undone_exactly(self):
        ticket = self.gate('runtime_set', {'chat': {'port': 2222}})
        (self.root / 'config/llm_runtime.toml').write_text('[chat]\nport = 2222\n')
        self.finish(ticket)
        listed = [u for u in self.a.undoable() if u['request_id'] == ticket['request_id']]
        self.assertEqual(listed[0]['reversible'], 'files')
        self.assertTrue(listed[0]['configuration_only'])
        self.a.admin('autonomy_undo', dict(request_id=ticket['request_id']), None)
        self.assertEqual((self.root / 'config/llm_runtime.toml').read_text(), '[chat]\nport = 1111\n')
        self.assertEqual(self.row(ticket['request_id'])['status'], 'undone')

    def test_ollama_server_change_is_now_file_backed(self):
        ticket = self.gate('ollama_add_server', {'address': 'http://b'})
        (self.root / 'config/settings_panel.toml').write_text('[ollama]\nservers = ["http://a", "http://b"]\n')
        self.finish(ticket)
        self.a.admin('autonomy_undo', dict(request_id=ticket['request_id']), None)
        self.assertEqual((self.root / 'config/settings_panel.toml').read_text(), '[ollama]\nservers = ["http://a"]\n')

    def test_exact_inverse_is_returned_then_confirmed(self):
        ticket = self.gate('connection_start', {'id': 'c1'})
        self.finish(ticket)
        listed = [u for u in self.a.undoable() if u['request_id'] == ticket['request_id']]
        self.assertEqual(listed[0]['reversible'], 'inverse')
        plan = self.a.admin('autonomy_undo', dict(request_id=ticket['request_id']), None)
        self.assertEqual(plan['inverse'], [{'tool': 'connection_stop', 'args': {'id': 'c1'}}])
        self.assertEqual(self.row(ticket['request_id'])['status'], 'undoing')
        again = self.a.admin('autonomy_undo', dict(request_id=ticket['request_id']), None)   # an interrupted undo can be asked again: same steps, no new snapshot
        self.assertEqual(again['inverse'], plan['inverse'])
        self.a.admin('autonomy_undo_done', dict(request_id=ticket['request_id']), None)
        self.assertEqual(self.row(ticket['request_id'])['status'], 'undone')
        self.assertTrue(self.a.admin('autonomy_undo_done', dict(request_id=ticket['request_id']), None)['already_recorded'])

    def test_inverse_built_from_the_captured_before_state(self):
        before = {'entries': [{'id': 'a', 'kind': 'tts', 'selected': True}, {'id': 'b', 'kind': 'tts', 'selected': False}]}
        ticket = self.gate('voice_select', {'id': 'b'}, before)
        self.finish(ticket)
        plan = self.a.admin('autonomy_undo', dict(request_id=ticket['request_id']), None)
        self.assertEqual(plan['inverse'], [{'tool': 'voice_select', 'args': {'id': 'a'}}])

    def test_unsupported_action_is_not_offered_and_says_why(self):
        ticket = self.gate('memory_reindex', {})
        self.finish(ticket)
        self.assertEqual([u for u in self.a.undoable() if u['request_id'] == ticket['request_id']], [])
        with self.assertRaises(ValueError) as caught: self.a.admin('autonomy_undo', dict(request_id=ticket['request_id']), None)
        self.assertIn('No exact undo exists', str(caught.exception)); self.assertIn('derived search data', str(caught.exception))

    def test_effect_endpoint_tells_the_bridge_what_to_capture(self):
        self.assertEqual(self.a.admin('autonomy_effect', dict(tool='voice_agent_set'), None)['capture'], {'tool': 'voice_status', 'args': {}})
        self.assertIsNone(self.a.admin('autonomy_effect', dict(tool='runtime_set'), None)['capture'])
        self.assertEqual(self.a.admin('autonomy_effect', dict(tool='workspace_delete'), None)['reversible'], 'none')

    def age(self, request, seconds=1000):
        with self.store.db() as db: db.execute('UPDATE autonomy_requests SET created=? WHERE id=?', (time.time() - seconds, request))

    def test_orphan_unchanged_files_become_failed(self):
        ticket = self.gate('runtime_set', {'chat': {'port': 3333}})          # the process "dies" here: no finish
        self.assertEqual(self.a.reconcile_orphans(), [], 'a fresh transaction is left alone')
        self.age(ticket['request_id'])
        self.assertEqual(self.a.reconcile_orphans(), [(ticket['request_id'], 'failed')])
        self.assertEqual(self.row(ticket['request_id'])['status'], 'failed')

    def test_orphan_changed_files_become_uncertain_and_can_be_undone(self):
        ticket = self.gate('runtime_set', {'chat': {'port': 4444}})
        (self.root / 'config/llm_runtime.toml').write_text('[chat]\nport = 4444\n')   # the effect happened, the end was never recorded
        self.age(ticket['request_id'])
        self.assertEqual(self.a.reconcile_orphans(), [(ticket['request_id'], 'uncertain')])
        self.assertTrue([u for u in self.a.undoable() if u['request_id'] == ticket['request_id']][0]['uncertain'])
        self.a.admin('autonomy_undo', dict(request_id=ticket['request_id']), None)
        self.assertEqual((self.root / 'config/llm_runtime.toml').read_text(), '[chat]\nport = 1111\n')

    def test_orphan_without_files_is_uncertain_not_replayed(self):
        ticket = self.gate('connection_start', {'id': 'c9'})
        self.age(ticket['request_id'])
        self.assertEqual(self.a.reconcile_orphans(), [(ticket['request_id'], 'uncertain')])
        self.assertEqual(self.a.reconcile_orphans(), [], 'reconciled once, never again')
        self.assertFalse(self.a.current()['stopped'], 'no emergency stop is raised for an orphan')

    def test_policy_changes_are_not_touched_by_this_path(self):
        now = time.time()
        with self.store.db() as db:
            db.execute('INSERT INTO autonomy_requests VALUES(?,?,?,?,?,?,?,?,?,?,?,?)', ('p' * 32, 'owner', '', 'native-policy', 'p' * 32, 'autonomy_permissions_set', 'executing', now - 5000, now, 0, self.store.seal('p' * 32, 'autonomy', dict()), 1))
        self.assertEqual(self.a.reconcile_orphans(), [], 'native-policy orphans are resolved by recover_permissions, never here')

if __name__ == '__main__':
    unittest.main()
