# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
import pathlib, sys, unittest
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import autonomy_effects as fx

NATIVE = {'runtime_set': ['config/llm_runtime.toml'], 'memory_set': ['config/hermes_adapter.toml', 'config/settings_panel.toml']}

class Effects(unittest.TestCase):
    def test_exact_opposites(self):
        self.assertEqual(fx.plan_inverse('connection_start', {'id': 'c1'}), [{'tool': 'connection_stop', 'args': {'id': 'c1'}}])
        self.assertEqual(fx.plan_inverse('connection_stop', {'id': 'c1'}), [{'tool': 'connection_start', 'args': {'id': 'c1'}}])
        self.assertEqual(fx.plan_inverse('modelpaths_add', {'path': '/models'}), [{'tool': 'modelpaths_remove', 'args': {'path': '/models'}}])
        self.assertEqual(fx.plan_inverse('modelfind_map', {'found': {'id': 'm1', 'path': '/x'}}), [{'tool': 'modelfind_unmap', 'args': {'id': 'm1'}}])
        self.assertEqual(fx.plan_inverse('voice_install', {'id': 'v'}), [{'tool': 'voice_remove', 'args': {'id': 'v'}}])

    def test_missing_arguments_give_no_plan(self):
        self.assertIsNone(fx.plan_inverse('connection_start', {}))
        self.assertIsNone(fx.plan_inverse('connection_start', {'id': ''}))
        self.assertIsNone(fx.plan_inverse('modelfind_map', {'found': {}}))
        self.assertIsNone(fx.plan_inverse('connection_start', 'not a dict'))

    def test_voice_select_puts_back_the_previous_voice_of_the_same_kind(self):
        before = {'entries': [{'id': 'a', 'kind': 'tts', 'selected': True}, {'id': 'b', 'kind': 'tts', 'selected': False}, {'id': 's', 'kind': 'stt', 'selected': True}]}
        self.assertEqual(fx.plan_inverse('voice_select', {'id': 'b'}, before), [{'tool': 'voice_select', 'args': {'id': 'a'}}])
        self.assertEqual(fx.plan_inverse('voice_select', {'id': 's'}, before), [{'tool': 'voice_select', 'args': {'id': 's'}}])
        self.assertIsNone(fx.plan_inverse('voice_select', {'id': 'b'}, {'entries': [{'id': 'b', 'kind': 'tts', 'selected': False}]}), 'nothing was selected before: no honest inverse')
        self.assertIsNone(fx.plan_inverse('voice_select', {'id': 'b'}, None))

    def test_voice_toggles_and_agent_voice(self):
        self.assertEqual(fx.plan_inverse('voice_noncommercial', {'accepted': True}, {'show_noncommercial': False}), [{'tool': 'voice_noncommercial', 'args': {'accepted': False}}])
        self.assertEqual(fx.plan_inverse('voice_chatterbox_set', {'url': 'http://n'}, {'chatterbox': {'url': 'http://old'}}), [{'tool': 'voice_chatterbox_set', 'args': {'url': 'http://old'}}])
        self.assertEqual(fx.plan_inverse('voice_chatterbox_set', {'url': 'http://n'}, {}), [{'tool': 'voice_chatterbox_set', 'args': {'url': ''}}])
        self.assertEqual(fx.plan_inverse('voice_agent_set', {'agent': 'agent-a', 'id': 'k'}, {'state': {'agents': {'agent-a': {'id': 'old'}}}}), [{'tool': 'voice_agent_set', 'args': {'agent': 'agent-a', 'id': 'old'}}])
        self.assertEqual(fx.plan_inverse('voice_agent_set', {'agent': 'agent-a', 'id': 'k'}, {'state': {}}), [{'tool': 'voice_agent_set', 'args': {'agent': 'agent-a', 'id': ''}}])

    def test_unmap_remaps_the_exact_record(self):
        found = {'id': 'm1', 'path': '/x', 'name': 'n'}
        self.assertEqual(fx.plan_inverse('modelfind_unmap', {'id': 'm1'}, {'mapped': [{'found': found}]}), [{'tool': 'modelfind_map', 'args': {'found': found}}])
        self.assertIsNone(fx.plan_inverse('modelfind_unmap', {'id': 'zz'}, {'mapped': [{'found': found}]}))

    def test_nothing_unsupported_is_advertised_as_reversible(self):
        for tool in ('memory_reindex', 'ontology_build', 'workspace_delete', 'chatbot_delete', 'agent_activate', 'memory_view_delete', 'knowledge_remove', 'knowledge_add'):
            d = fx.describe(tool, NATIVE)
            self.assertEqual(d['reversible'], 'none', tool)
            self.assertTrue(d['how'], tool)
            self.assertIsNone(fx.plan_inverse(tool, {'id': 'x'}, {}), tool)
        self.assertEqual(fx.describe('some_future_tool', NATIVE)['reversible'], 'none')

    def test_describe_kinds(self):
        self.assertEqual(fx.describe('runtime_set', NATIVE)['reversible'], 'files')
        self.assertIn('config/llm_runtime.toml', fx.describe('runtime_set', NATIVE)['how'])
        self.assertEqual(fx.describe('connection_start', NATIVE)['reversible'], 'inverse')
        self.assertEqual(fx.describe('voice_select', NATIVE)['reversible'], 'inverse')

    def test_every_capture_tool_has_a_planner(self):
        for tool in fx.CAPTURE:
            self.assertIsNotNone(fx.plan_inverse(tool, {'id': 'x', 'agent': 'a', 'accepted': True, 'url': 'u'}, {'entries': [{'id': 'x', 'kind': 'tts', 'selected': True}], 'mapped': [{'found': {'id': 'x'}}]}), tool)

    def test_orphan_classification(self):
        same = lambda a, b: a == b
        self.assertEqual(fx.classify_orphan([{'d': 1}], [{'d': 1}], same)[0], 'failed')
        self.assertEqual(fx.classify_orphan([{'d': 1}], [{'d': 2}], same)[0], 'uncertain')
        self.assertEqual(fx.classify_orphan([{'d': 1}], [], same)[0], 'uncertain')

if __name__ == '__main__':
    unittest.main()
