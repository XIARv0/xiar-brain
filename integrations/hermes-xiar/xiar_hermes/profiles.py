# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Admin-controlled external profile registry. No identity supplied by the model.
Every (Agent, verified fork user, business) has its own native Hermes home, memory
credential, session/kanban/workspace and decision state. All JSON is private;
public DTOs never contain credentials. Native source and existing homes untouched.
"""
from __future__ import annotations
import contextlib,copy,hashlib,json,os,pathlib,re,secrets,tempfile,tomllib
from .config import AdapterConfig,xiar_root
SLUG=re.compile(r"[a-z0-9][a-z0-9_-]{0,62}\Z")

def slug(value):
    if not isinstance(value,str) or not SLUG.fullmatch(value):raise ValueError('Normalized identity required')
    return value

def private_dir(path):
    if path.is_symlink():raise ValueError('Profile directories cannot be symlinks')
    path.mkdir(parents=True,exist_ok=True,mode=0o700);os.chmod(path,0o700)

def atomic(path,value):
    private_dir(path.parent)
    fd,temp=tempfile.mkstemp(prefix='.profile-',dir=path.parent)
    try:
        with os.fdopen(fd,'w',encoding='utf-8') as stream:
            os.fchmod(stream.fileno(),0o600);stream.write(value);stream.flush();os.fsync(stream.fileno())
        os.replace(temp,path)
        fd=os.open(path.parent,os.O_RDONLY)
        try:os.fsync(fd)
        finally:os.close(fd)
    finally:
        if os.path.exists(temp):os.unlink(temp)

def save(path,data):atomic(path,json.dumps(data,sort_keys=True,ensure_ascii=False)+'\n')

SHARED_REGISTRY=pathlib.Path('/etc/xiar')
# Where the root broker keeps the Agent registry, every private home and every capability. Fixed here on purpose: the root process never takes this location from a
# configuration file that the owner's Linux user can edit.
CONTROL_AGENTS=pathlib.Path('/var/lib/xiar-agent-control/agents')

GRANTS_GROUP='xiar'   # the memory API runs as the standing Xiar's user
GRANTS_MODE=0o640
def publish(path,data):
    """Write the hashed memory-grant registry (version 2): atomically, root-owned, readable only by root and the standing Xiar's group (0640 root:xiar). It holds
    only SHA-256 digests of the private capabilities, so the memory API (which runs as the standing Xiar's user) can verify a token but can neither recover it nor
    add or change a grant; no Agent user can even read which grants exist."""
    import grp
    gid=grp.getgrnam(GRANTS_GROUP).gr_gid
    parent=path.parent;info=parent.stat()
    if parent.is_symlink() or info.st_uid!=0 or info.st_mode&0o022:raise ValueError('Grant registry directory must be root-owned and not writable by others')
    fd,temp=tempfile.mkstemp(prefix='.grants-',dir=parent)
    try:
        with os.fdopen(fd,'w',encoding='utf-8') as stream:
            os.fchown(stream.fileno(),0,gid);os.fchmod(stream.fileno(),GRANTS_MODE);stream.write(json.dumps(data,sort_keys=True,ensure_ascii=False)+'\n');stream.flush();os.fsync(stream.fileno())
        os.replace(temp,path)
    finally:
        if os.path.exists(temp):os.unlink(temp)
def read(path,default=None):
    if not path.exists():return default
    if path.is_symlink() or path.stat().st_mode & 0o077:raise ValueError('Private profile metadata permissions invalid')
    return json.loads(path.read_text(encoding='utf-8'))

class Profiles:
    def __init__(self,root=None):
        self.root=pathlib.Path(root or xiar_root()).resolve()
        self.cfg=tomllib.loads((self.root/'config/agent_profiles.toml').read_text(encoding='utf-8-sig'))
        self.as_root=os.geteuid()==0
        self.base=CONTROL_AGENTS if self.as_root else self._path(self.cfg['profiles']['root']);self.homes=self.base/'profiles'
        self.templates=self._path(self.cfg['profiles']['templates'])
        self.index=self.base/'registry.json'
        auth=tomllib.loads((self.root/'config/memory_api.toml').read_text(encoding='utf-8-sig'))['auth']
        raw=auth.get('dynamic_grants_file','data/agents/memory-grants.json');shared=pathlib.Path(raw)
        # a registry directly inside /etc/xiar is the hashed, root-owned one (version 2); a path under data/ is the older private plaintext one (version 1)
        self.grants=SHARED_REGISTRY/'memory-grants.json' if self.as_root else (shared if shared.is_absolute() and shared.parent==SHARED_REGISTRY else self._path(raw))
    def _path(self,value):
        p=(self.root/value).resolve()
        if not p.is_relative_to(self.root/'data'):raise ValueError('Profile state must stay under XIAR data')
        return p
    @contextlib.contextmanager
    def locked(self):
        import fcntl
        private_dir(self.base)
        fd=os.open(self.base/'registry.lock',os.O_CREAT|os.O_RDWR|os.O_NOFOLLOW,0o600)
        try:
            fcntl.flock(fd,fcntl.LOCK_EX);yield
        finally:fcntl.flock(fd,fcntl.LOCK_UN);os.close(fd)
    def state(self):
        state=read(self.index,{'version':1,'agents':{},'scopes':{}})
        if state.get('version')!=1 or not isinstance(state.get('agents'),dict) or not isinstance(state.get('scopes'),dict):raise ValueError('Unknown profile registry format')
        return state
    def list(self):
        state=self.state()
        return {'agents':list(state['agents'].values()),'scopes':[self.public(row) for row in state['scopes'].values()]}
    @staticmethod
    def public(row):return {k:row[k] for k in ('profile','agent','user','business','memory_mode','active','enabled')}
    def upsert(self,data):
        allowed={'id','title','businesses','memory_mode','run_mode','template','enabled','linux_user','power'}
        if set(data)-allowed:raise ValueError('Unknown Agent setting')
        agent=slug(data['id'])
        if agent in ('default','hermes','bot'):raise ValueError('Reserved Agent identity')
        with self.locked():
            state=self.state();previous=state['agents'].get(agent,{})
            value={**dict(id=agent,title=agent,businesses=[],memory_mode='hermes_only',runtime_managed=True,runtime_ready=False,run_mode='always_on',template='',enabled=True,linux_user='',power='standard'),**previous,**data}
            if value['memory_mode'] not in ('none','hermes_only','xiar_memory') or value['run_mode'] not in ('always_on','on_demand'):raise ValueError('Invalid memory or run mode')
            if value['power'] not in ('standard','full'):raise ValueError('Agent power must be standard or full')
            if not isinstance(value['enabled'],bool):raise ValueError('Agent enabled must be boolean')
            if not isinstance(value['title'],str) or not 0<len(value['title'])<=128:raise ValueError('Invalid Agent title')
            if not isinstance(value['businesses'],list) or not value['businesses'] or len(value['businesses'])>128:raise ValueError('Choose explicitly granted businesses')
            value['businesses']=sorted({slug(b) for b in value['businesses']})
            if value['template']:slug(value['template'])
            value['linux_user']=value['linux_user'] or 'dedicated'   # every Agent runs as its own Linux user
            if value['linux_user']:
                expected='xiar-a-'+hashlib.sha256(agent.encode()).hexdigest()[:16]
                if value['linux_user'] not in ('dedicated',expected):raise ValueError('Select dedicated isolation; arbitrary existing Linux accounts are forbidden')
                value['linux_user']=expected
            if agent not in state['agents'] and len(state['agents'])>=self.cfg['profiles']['max_agents']:raise ValueError('Agent registry full')
            # Deny grants FIRST on a mode/business/enable change; no stale token can
            # keep writing while native profiles are being reconfigured.
            state['agents'][agent]=value
            self._write_grants(state,exclude_agent=agent)
            for row in state['scopes'].values():
                if row['agent']==agent:
                    row['memory_mode']=value['memory_mode'];row['enabled']=value['enabled'] and row['business'] in value['businesses'] and (not value['linux_user'] or value.get('runtime_ready',False))
                    row['active']=row['enabled'] and (value['run_mode']=='always_on' or (previous.get('run_mode')==value['run_mode'] and row['active']))
                    self._configure(row,value)
            save(self.index,state);self._write_grants(state)
            return value
    def provision(self,agent,verified_user,business):
        agent=slug(agent);business=slug(business)
        if not isinstance(verified_user,str) or not 0<len(verified_user)<=256:raise ValueError('Verified fork user required')
        user='u-'+hashlib.sha256(verified_user.encode()).hexdigest()[:40]
        profile='a-'+agent[:16]+'-'+hashlib.sha256((agent+'\0'+user+'\0'+business).encode()).hexdigest()[:32]
        with self.locked():
            state=self.state();cfg=state['agents'].get(agent)
            if not cfg or not cfg['enabled'] or business not in cfg['businesses']:raise ValueError('Agent/business not granted')
            row=state['scopes'].get(profile)
            if row:
                self.validate(row)
                if not row.get('enabled'):raise ValueError('Profile disabled')
                self._write_grants(state)
                return self.public(row)
            if len(state['scopes'])>=self.cfg['profiles']['max_scopes']:raise ValueError('Private scope registry full')
            row=dict(profile=profile,agent=agent,user=user,business=business,memory_mode=cfg['memory_mode'],enabled=not cfg['linux_user'] or cfg.get('runtime_ready',False),active=(not cfg['linux_user'] or cfg.get('runtime_ready',False)) and cfg['run_mode']=='always_on')
            home=self.home(profile);private_dir(home)
            marker=home/'.xiar-identity.json'
            identity={k:row[k] for k in ('profile','agent','user','business')}
            existing=read(marker)
            if existing is not None and existing!=identity:raise ValueError('Private home belongs to another identity')
            if existing is None and any(home.iterdir()):raise ValueError('Refusing unbound nonempty profile home')
            save(marker,identity)
            for name in ('workspace','skills','memories','sessions','state'):private_dir(home/name)
            # Do not clone profiles or import keys, config, memory or session data.
            if cfg['template']:
                source=(self.templates/slug(cfg['template'])).resolve()
                if not source.is_relative_to(self.templates) or source.is_symlink():raise ValueError('Template outside template root')
                for name in ('SOUL.md','AGENTS.md'):
                    p=source/name
                    if p.exists():
                        if p.is_symlink() or p.stat().st_size>self.cfg['profiles']['max_template_bytes']:raise ValueError('Unsafe template file')
                        atomic(home/name,p.read_text(encoding='utf-8'))
            if not (home/'memory-api-token').exists():atomic(home/'memory-api-token',secrets.token_hex(32)+'\n')
            if not (home/'.env').exists():atomic(home/'.env','API_SERVER_KEY='+secrets.token_hex(32)+'\nHERMES_WRITE_SAFE_ROOT='+str(home)+'\n')
            self._configure(row,cfg);state['scopes'][profile]=row
            # Commit scope before credential publication: interruption leaves a
            # denied/incomplete scope, never a widened grant. Retry repairs it.
            save(self.index,state);self._write_grants(state)
            return self.public(row)
    def home(self,profile):return self.homes/slug(profile)
    def validate(self,row):
        home=self.home(row['profile'])
        if home.is_symlink() or read(home/'.xiar-identity.json')!={k:row[k] for k in ('profile','agent','user','business')}:raise ValueError('Profile identity marker mismatch')
        return home
    def _configure(self,row,cfg):
        home=self.validate(row)
        runtime=tomllib.loads((self.root/'config/llm_runtime.toml').read_text(encoding='utf-8-sig'))['chat']
        if runtime['host'] not in ('127.0.0.1','localhost','::1'):raise ValueError('Native default connection must be loopback')
        shared=[str((self.root/p).resolve()) for p in self.cfg['shared']['skills']]
        for path in shared:
            if not pathlib.Path(path).is_relative_to(self.root/'integrations'):raise ValueError('Shared skill path outside integrations')
        native=dict(model=dict(provider='custom',base_url=f"http://{runtime['host']}:{int(runtime['port'])}/v1",default=runtime['served_name'],context_length=runtime['context_length']),plugins=dict(enabled=['xiar']),skills=dict(external_dirs=shared),memory=dict(memory_enabled=cfg['memory_mode']=='hermes_only',user_profile_enabled=cfg['memory_mode']=='hermes_only'),terminal=dict(cwd=str(home/'workspace')),gateway=dict(standalone=False,parked=not row['active']))
        # Separate purpose-scoped local token, never a stock-home key import.
        if not (home/'model-router-token').exists():atomic(home/'model-router-token',secrets.token_hex(32)+'\n')
        from .model_overlay import apply as apply_model_transport
        native=apply_model_transport(native,self.root,home,{k:row[k] for k in ('profile','agent','user','business')})
        # JSON is YAML 1.2, parsed by native Hermes, with no copied environment/key.
        atomic(home/'config.yaml',json.dumps(native,indent=2)+'\n')
        atomic(home/'profile.yaml',json.dumps(dict(name=row['profile'],description=cfg['title']))+'\n')
        adapter=tomllib.loads((self.root/'config/hermes_adapter.toml').read_text(encoding='utf-8-sig'))
        adapter['identity']=dict(agent_id=row['agent'],end_user_id=row['user'])
        adapter['profile']=dict(home=str(home),read_only_roots=[*shared,str((self.root/self.cfg['shared']['mental_models']).resolve())])
        adapter['brain']['enabled']=row['agent']=='xiar'
        adapter['memory'].update(enabled=cfg['memory_mode']=='xiar_memory',token_file=str(home/'memory-api-token'),default_business=row['business'],auto_recall=cfg['memory_mode']=='xiar_memory')
        adapter['routing'].update(state_dir=str(home/'state'),businesses=[row['business']])
        if cfg['memory_mode']!='xiar_memory':
            for job in ('memory_routing','retrieval_strategy','correction_detection'):adapter['laya']['jobs'][job]=False
        if row['agent']!='xiar':adapter['gate']['directive']='off'
        save(home/'adapter.json',adapter)
    def _write_grants(self,state,exclude_agent=None):
        grants=[]
        for row in state['scopes'].values():
            if row['agent']==exclude_agent or not row.get('enabled') or row['memory_mode']!='xiar_memory':continue
            home=self.validate(row)
            # Internal application provisioning only. Never returned/logged.
            token=(home/'memory-api-token').read_text().strip()
            if not re.fullmatch('[a-f0-9]{64}',token):raise ValueError('Invalid private token format')
            if self.grants.parent==SHARED_REGISTRY:grants.append(dict(token_sha256=hashlib.sha256(token.encode()).hexdigest(),agent=row['agent'],user=row['user'],businesses=[row['business']]))
            else:grants.append(dict(token=token,agent=row['agent'],user=row['user'],businesses=[row['business']]))
        if self.grants.parent==SHARED_REGISTRY:publish(self.grants,dict(version=2,grants=grants))
        else:save(self.grants,dict(version=1,grants=grants))
    def activate(self,profile):
        with self.locked():
            state=self.state();row=state['scopes'].get(slug(profile))
            if not row or not row.get('enabled'):raise ValueError('Unknown or disabled private profile')
            self.validate(row)
            if not row['active']:
                row['active']=True;self._configure(row,state['agents'][row['agent']]);save(self.index,state)
            return self.public(row)
    def reset_on_demand(self):
        # Start-when-used is reset across gateway restarts; persisted session and
        # memory stay private/intact. No new idle lifetime is silently invented.
        if not self.index.exists():return
        with self.locked():
            state=self.state();changed=False
            for row in state['scopes'].values():
                cfg=state['agents'][row['agent']]
                if not cfg.get('runtime_managed') and cfg['run_mode']=='on_demand' and row['active']:
                    row['active']=False;self._configure(row,cfg);changed=True
            if changed:save(self.index,state)
    def rows(self,active=False):
        for row in self.state()['scopes'].values():
            if row.get('enabled') and (not active or row['active']):
                self.validate(row);yield row
    def scoped_config(self,home):
        home=pathlib.Path(home).resolve()
        if not home.is_relative_to(self.homes):return None
        row=self.state()['scopes'].get(home.name)
        if not row or not row.get('enabled') or self.validate(row).resolve()!=home:raise ValueError('Unregistered private profile')
        cfg=AdapterConfig(read(home/'adapter.json'),self.root)
        if cfg.get('profile.home')!=str(home) or cfg.get('memory.token_file')!=str(home/'memory-api-token') or cfg.get('memory.default_business')!=row['business']:raise ValueError('Adapter scope paths mismatch')
        if cfg.get('identity.agent_id')!=row['agent'] or cfg.get('identity.end_user_id')!=row['user']:raise ValueError('Adapter identity mismatch')
        if row['agent']!='xiar' and cfg.get('brain.enabled'):raise ValueError('Brain belongs only to Xiar')
        return cfg
