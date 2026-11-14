# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Prepared isolation proof. RUN ONLY in the final integrated acceptance pass.
No model inference, live user records or existing Hermes files are changed. Native
resolver is overlaid only in this short-lived proof process. Live HTTP/token/header,
file-tool, fork group and end-to-end turn proofs remain the integrated runner's job.
"""
import json,os,pathlib,shutil,sys,tempfile,unittest
ROOT=pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'integrations/hermes-xiar'))
from xiar_hermes.profiles import Profiles,save
from xiar_hermes.profile_guard import limits
class ProfileProof(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(prefix='xiar-profile-proof-');self.addCleanup(self.tmp.cleanup)
        self.root=pathlib.Path(self.tmp.name);(self.root/'config').mkdir()
        for name in ('agent_profiles.toml','memory_api.toml','hermes_adapter.toml','llm_runtime.toml'):
            shutil.copyfile(ROOT/'config'/name,self.root/'config'/name)
        self.p=Profiles(self.root)
        self.p.upsert(dict(id='alpha',businesses=['acme','beta'],memory_mode='xiar_memory'))
    def scope(self,user='alice',business='acme',agent='alpha'):return self.p.provision(agent,user,business)
    def grants(self):return json.loads(self.p.grants.read_text())['grants']
    def test_stable_scope(self):self.assertEqual(self.scope(),self.scope())
    def test_private_user_homes(self):self.assertNotEqual(self.scope()['profile'],self.scope('bob')['profile'])
    def test_private_business_homes(self):self.assertNotEqual(self.scope()['profile'],self.scope(business='beta')['profile'])
    def test_separate_agent_homes(self):
        self.p.upsert(dict(id='bravo',businesses=['acme'],memory_mode='xiar_memory'))
        self.assertNotEqual(self.scope()['profile'],self.scope(agent='bravo')['profile'])
    def test_marker_and_permissions(self):
        row=self.scope();home=self.p.home(row['profile']);self.assertEqual(home.stat().st_mode&0o777,0o700)
        self.assertEqual((home/'memory-api-token').stat().st_mode&0o777,0o600)
        self.assertEqual(self.p.validate(self.p.state()['scopes'][row['profile']]),home)
    def test_exact_token_grants(self):
        row=self.scope();grants=self.grants();self.assertEqual(len(grants),1)
        self.assertTrue(grants[0]['agent']==row['agent'] and grants[0]['user']==row['user'] and grants[0]['businesses']==['acme'])
        self.assertFalse('token' in row)
    def test_none_revokes_grant(self):
        row=self.scope();self.p.upsert(dict(id='alpha',businesses=['acme'],memory_mode='none'))
        self.assertEqual(len(self.grants()),0)
        cfg=self.p.scoped_config(self.p.home(row['profile']));self.assertFalse(cfg.get('memory.enabled'))
        self.assertFalse(json.loads((self.p.home(row['profile'])/'config.yaml').read_text())['memory']['memory_enabled'])
    def test_hermes_only_revokes_rust(self):
        row=self.scope();self.p.upsert(dict(id='alpha',businesses=['acme'],memory_mode='hermes_only'))
        self.assertEqual(len(self.grants()),0)
        self.assertTrue(json.loads((self.p.home(row['profile'])/'config.yaml').read_text())['memory']['memory_enabled'])
    def test_disable_revokes_grant(self):
        self.scope();self.p.upsert(dict(id='alpha',businesses=['acme'],enabled=False));self.assertEqual(len(self.grants()),0)
        with self.assertRaises(ValueError):self.scope()
    def test_business_change_revokes_old_scope(self):
        row=self.scope();self.p.upsert(dict(id='alpha',businesses=['beta']));self.assertEqual(len(self.grants()),0)
        with self.assertRaises(ValueError):self.p.scoped_config(self.p.home(row['profile']))
    def test_nonxiar_brain_disabled(self):
        row=self.scope();self.assertFalse(self.p.scoped_config(self.p.home(row['profile'])).get('brain.enabled'))
    def test_tampered_marker_refused(self):
        row=self.scope();save(self.p.home(row['profile'])/'.xiar-identity.json',dict(agent='bravo'))
        with self.assertRaises(ValueError):self.scope()
    def test_ungranted_business_refused(self):
        with self.assertRaises(ValueError):self.scope(business='outside')
    def test_traversal_refused(self):
        with self.assertRaises(ValueError):self.scope(agent='../alpha')
    def test_pending_dedicated_user_is_not_faked(self):
        self.p.upsert(dict(id='alpha',businesses=['acme'],linux_user='dedicated'))
        self.assertFalse(self.scope()['enabled'])
        self.assertEqual(len(self.grants()),0)
    def test_demand_activation(self):
        self.p.upsert(dict(id='alpha',businesses=['acme'],run_mode='on_demand'))
        row=self.scope();self.assertFalse(row['active']);self.assertTrue(self.p.activate(row['profile'])['active'])
    def test_folder_limits(self):
        row=self.scope();home=self.p.home(row['profile']);cfg=self.p.scoped_config(home)
        self.assertIsNone(limits(cfg,'write_file',dict(path='work.txt')))
        self.assertEqual(limits(cfg,'write_file',dict(path=str(self.root/'outside')))['action'],'block')
        self.assertEqual(limits(cfg,'read_file',dict(path=str(home/'.env')))['action'],'block')
        self.assertEqual(limits(cfg,'write_file',dict(path=str(home/'adapter.json')))['action'],'block')
    def test_z_native_resolver_and_context_binding(self):
        # Last case; isolated proof process exits after removing its overlay root.
        row=self.scope();old=os.environ.get('XIAR_ROOT');os.environ['XIAR_ROOT']=str(self.root)
        self.addCleanup(lambda:os.environ.pop('XIAR_ROOT',None) if old is None else os.environ.__setitem__('XIAR_ROOT',old))
        import tomllib,types
        source=pathlib.Path(tomllib.loads((self.root/'config/agent_profiles.toml').read_text())['native']['source']).expanduser()
        sys.path.insert(0,str(source))
        from hermes_cli import profiles as native
        default=native.get_profile_dir('default')
        from xiar_hermes.native_profiles import install
        state=self.p.state();state['agents']['alpha']['runtime_managed']=False;save(self.p.index,state)
        install();self.assertEqual(native.get_profile_dir('default'),default)
        self.assertEqual(native.get_profile_dir(row['profile']),self.p.home(row['profile']))
        from hermes_constants import set_hermes_home_override,reset_hermes_home_override
        from xiar_hermes.plugin import XiarAdapter,profile_hooks
        cfg=self.p.scoped_config(self.p.home(row['profile']));cfg.raw['mental_models']['enabled']=False;cfg.raw['laya']['enabled']=False
        token=set_hermes_home_override(str(self.p.home(row['profile'])))
        try:a=XiarAdapter(cfg=cfg,brain=types.SimpleNamespace(),memory=types.SimpleNamespace(),laya=types.SimpleNamespace(),background=False)
        finally:reset_hermes_home_override(token)
        token=set_hermes_home_override(str(self.root/'foreign-home'))
        try:self.assertEqual(a.pre_tool_call(tool_name='write_file',args={'path':'a'})['action'],'block')
        finally:reset_hermes_home_override(token)
        self.assertNotIn('post_approval_response',profile_hooks(cfg));self.assertNotIn('post_tool_call',profile_hooks(cfg))
if __name__=='__main__':unittest.main()
