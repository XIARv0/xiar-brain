# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Read-only root-published binding for a separate-UID Agent mount namespace."""
import json,os,pathlib,stat
from .config import AdapterConfig
class RuntimeProfiles:
    def __init__(self):
        self.path=pathlib.Path(os.environ['XIAR_RUNTIME_MANIFEST'])
        st=self.path.stat()
        if st.st_uid!=0 or st.st_mode&0o022 or self.path.is_symlink():raise ValueError('Runtime manifest must be root-owned and read-only')
        self.manifest=json.loads(self.path.read_text());self.agent=self.manifest['agent']
        if self.manifest.get('version')!=1:raise ValueError('Unknown runtime binding')
    def state(self):
        fresh=RuntimeProfiles().manifest
        return {'agents':{fresh['agent']:fresh['config']},'scopes':{r['profile']:r for r in fresh['scopes']}}
    def validate(self,row):
        home=pathlib.Path('/var/lib/xiar-agent')
        if home.is_symlink() or not home.is_dir():raise ValueError('Invalid private runtime home')
        marker=json.loads((home/'.xiar-identity.json').read_text())
        if marker!={k:row[k] for k in ('profile','agent','user','business')} or row['agent']!=self.agent:raise ValueError('Runtime identity mismatch')
        return home
    def rows(self,active=False):
        for row in self.state()['scopes'].values():
            if row.get('enabled') and (not active or row.get('active')):self.validate(row);yield row
    def scoped_config(self,home):
        home=pathlib.Path(home).resolve();row=self.state()['scopes'].get(self.manifest['profile'])
        if row is None or not row.get('enabled') or self.validate(row)!=home:raise ValueError('Unbound isolated profile')
        cfg=AdapterConfig(json.loads((home/'adapter.json').read_text()),pathlib.Path('/opt/xiar/project'))
        if cfg.get('identity.agent_id')!=row['agent'] or cfg.get('identity.end_user_id')!=row['user'] or cfg.get('memory.default_business')!=row['business'] or cfg.get('profile.home')!=str(home) or cfg.path('memory.token_file')!=home/'memory-api-token':raise ValueError('Runtime adapter scope mismatch')
        if row['agent']!='xiar' and cfg.get('brain.enabled'):raise ValueError('Only Xiar gets the brain')
        return cfg
