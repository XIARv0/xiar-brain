# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Root-only optional per-Agent Linux-user provisioner; no Hermes writes or key import.
Only fixed generated users, fixed mounts, fixed source and bounded loopback ports.
The privileged executable/module closure is installed root-owned outside the repo.
"""
import hashlib,json,os,pathlib,pwd,re,secrets,shutil,subprocess,time,urllib.request,tomllib
import paths as xiar_paths
from xiar_hermes.profiles import Profiles,slug
ROOT=xiar_paths.project_root();CONTROL=xiar_paths.control_dir()
NATIVE=xiar_paths.hermes_native()
OWNER=xiar_paths.owner()
MUTABLE=('workspace','skills','memories','sessions','state','logs','cron','kanban','state.db','state.db-wal','state.db-shm')
def username(agent):return 'xiar-a-'+hashlib.sha256(slug(agent).encode()).hexdigest()[:16]
def private(path):path.mkdir(parents=True,exist_ok=True,mode=0o700);os.chmod(path,0o700)
def atomic_root(path,data,mode=0o600,gid=0):
    path.parent.mkdir(parents=True,exist_ok=True,mode=0o700);tmp=path.with_name(path.name+'.new')
    fd=os.open(tmp,os.O_WRONLY|os.O_CREAT|os.O_TRUNC|os.O_NOFOLLOW,mode)
    with os.fdopen(fd,'wb') as stream:stream.write(data);stream.flush();os.fsync(stream.fileno())
    os.chmod(tmp,mode);uid,gid=xiar_paths.write_uids(gid);os.chown(tmp,uid,gid);os.replace(tmp,path)
def ownership(path,uid,gid):
    os.chown(path,uid,gid,follow_symlinks=False)
    if path.is_dir() and not path.is_symlink():
        for base,dirs,files in os.walk(path,followlinks=False):
            for name in dirs+files:os.chown(pathlib.Path(base)/name,uid,gid,follow_symlinks=False)
def state():return Profiles(ROOT).state()
def hermes_version():
    """The installed Hermes, as a small fingerprint (version plus a hash of two small files). The source is owner-writable, so nothing else is taken from it."""
    import re as _re
    parts=[];version='unknown'
    for name in ('pyproject.toml','COMPAT_MANIFEST.md'):
        try:
            fd=os.open(NATIVE/name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
            try:
                data=os.read(fd,262144)
                parts.append(data)
                if name=='pyproject.toml':
                    found=_re.search(rb'^version\s*=\s*"([0-9A-Za-z.+-]{1,32})"',data,_re.M)
                    if found:version=found.group(1).decode()
            finally:os.close(fd)
        except OSError:parts.append(b'')
    return dict(version=version,fingerprint=hashlib.sha256(b'\0'.join(parts)).hexdigest()[:12])
def unit_name(agent):return 'xiar-agent-'+slug(agent)+'.service'
def stop(agent):subprocess.run(['systemctl','stop',unit_name(agent)],check=False,capture_output=True,timeout=100)
def records():
    path=CONTROL/'accounts.json'
    return json.loads(path.read_text()) if path.exists() else {}

def account(agent,dedicated):
    private(CONTROL);all=records();name=username(agent) if dedicated else 'xiar'
    entry=all.get(agent,{})
    if dedicated and not entry.get('dedicated_user'):
        try:pwd.getpwnam(name)
        except KeyError:pass
        else:raise ValueError('Refusing to adopt an unrelated Linux account')
        subprocess.run(['/usr/sbin/useradd','--system','--no-create-home','--user-group','--home-dir','/var/lib/xiar-agent','--shell','/usr/sbin/nologin','--comment','XIAR Agent '+agent,name],check=True,capture_output=True)
        entry['dedicated_user']=name
    if dedicated and entry['dedicated_user']!=username(agent):raise ValueError('Account identity mismatch')
    info=pwd.getpwnam(name)
    if info.pw_uid==0 or (dedicated and info.pw_uid==OWNER.pw_uid):raise ValueError('Unsafe runtime UID')
    entry.update(user=name,uid=info.pw_uid,gid=info.pw_gid);entry.setdefault('profiles',{})
    all[agent]=entry;atomic_root(CONTROL/'accounts.json',json.dumps(all).encode())
    return entry

FULL_POWER_DROP=('TemporaryFileSystem=/run:ro','ProtectSystem=','ProtectHome=','PrivateDevices=','NoNewPrivileges=','ProtectKernelTunables=','AppArmorProfile=','ProtectKernelModules=','ProtectControlGroups=','RestrictSUIDSGID=','CapabilityBoundingSet=','InaccessiblePaths=')
def godseye_history_allowed(agent):
    """Gods Eye history (Phase 8.5): Xiar always; another Agent only when the owner listed it in config/godseye_streams.toml [access] history_agents."""
    if agent=='xiar':return True
    try:return agent in tomllib.loads((ROOT/'config/godseye_streams.toml').read_text()).get('access',{}).get('history_agents',[])
    except (OSError,ValueError):return False
def full_power_unit(text):
    """Full power Agent (owner decision 2026-11-11): same unit without the sandbox lines, so it can use sudo like Xiar."""
    return '\n'.join(line for line in text.splitlines() if not line.startswith(FULL_POWER_DROP))+'\n'
def sudo_grant(user,full):
    """Password-free sudo for a Full power Agent's own Linux user; removed again when it goes back to standard."""
    path=pathlib.Path('/etc/sudoers.d')/('xiar-agent-'+slug(user))
    if not full:
        if path.exists():path.unlink()
        return
    data=f'{user} ALL=(ALL:ALL) NOPASSWD: ALL\n'.encode()
    tmp=path.with_suffix('.tmp');tmp.write_bytes(data);os.chmod(tmp,0o440)
    if subprocess.run(['/usr/sbin/visudo','-cf',str(tmp)],capture_output=True).returncode!=0:tmp.unlink();raise ValueError('sudoers check failed')
    os.replace(tmp,path)
def profile_unit(profile):return 'xiar-profile-'+slug(profile)+'.service'
def stop(agent):
    import agent_monitor
    for profile in records().get(agent,{}).get('profiles',{}):
        agent_monitor.intent(profile,'administrator (Settings)','stop requested by the administrator or an approved Agent action')   # a requested stop is never restarted by the monitor
        subprocess.run(['systemctl','stop',profile_unit(profile)],check=False,capture_output=True,timeout=100)
def changed(path,data,uid,gid):
    if path.is_symlink():raise ValueError('Unsafe runtime file')
    if not path.exists() or path.read_bytes()!=data:
        path.write_bytes(data);os.chmod(path,0o600);os.chown(path,uid,gid)
def synchronize(agent,first,last):
    agent=slug(agent);p=Profiles(ROOT);current=p.state();cfg=current['agents'][agent]
    dedicated=bool(cfg['linux_user'])
    if dedicated and cfg['linux_user']!=username(agent):raise ValueError('Use this Agent generated Linux account')
    acc=account(agent,dedicated);all=records();acc=all[agent]
    used={s['port'] for a in all.values() for s in a.get('profiles',{}).values()}
    scopes=[row for row in current['scopes'].values() if row['agent']==agent]
    for row in scopes:
        profile=slug(row['profile']);original=p.validate(row);home=p.base/'isolated'/agent/profile
        if not original.resolve().is_relative_to(p.base) or not home.resolve().is_relative_to(p.base):raise ValueError('Private runtime escaped the Agent registry')
        if home.is_symlink():raise ValueError('Unsafe private home')
        private(home)
        if profile not in acc['profiles']:
            port=next((v for v in range(first,last+1) if v not in used),None)
            if port is None:raise ValueError('Configured private port range exhausted')
            acc['profiles'][profile]={'port':port};used.add(port)
        port=acc['profiles'][profile]['port']
        for name in MUTABLE:
            src=original/name;target=home/name
            if src.exists() and not target.exists():os.replace(src,target);ownership(target,acc['uid'],acc['gid'])
        for name in ('.xiar-identity.json','memory-api-token','model-router-token','.env','SOUL.md','AGENTS.md','profile.yaml'):
            src=original/name
            if src.exists():
                if src.is_symlink():raise ValueError('Unsafe control file')
                changed(home/name,src.read_bytes(),acc['uid'],acc['gid'])
        native=json.loads((original/'config.yaml').read_text());native['terminal']['cwd']='/var/lib/xiar-agent/workspace'
        native['skills']['external_dirs']=['/opt/xiar/project/integrations/hermes-xiar/skills'];native['gateway']={'multiplex_profiles':False}
        from xiar_hermes.model_overlay import apply as apply_model_transport
        native=apply_model_transport(native,ROOT,pathlib.Path('/var/lib/xiar-agent'),{k:row[k] for k in ('profile','agent','user','business')})
        changed(home/'config.yaml',json.dumps(native).encode(),acc['uid'],acc['gid'])
        adapter=json.loads((original/'adapter.json').read_text());dest='/var/lib/xiar-agent'
        adapter['profile']=dict(home=dest,read_only_roots=['/opt/xiar/project/integrations/hermes-xiar/skills','/opt/xiar/project/Brain/Shared/mental-models'])
        adapter['memory']['token_file']=dest+'/memory-api-token';adapter['routing']['state_dir']=dest+'/state'
        adapter['laya']['token_file']='$CREDENTIALS_DIRECTORY/laya-token';adapter['brain']['tokens_file']='$CREDENTIALS_DIRECTORY/brain-tokens.json'
        changed(home/'adapter.json',json.dumps(adapter).encode(),acc['uid'],acc['gid'])
        if home.stat().st_uid!=acc['uid']:ownership(home,acc['uid'],acc['gid'])
        manifest=dict(version=1,agent=agent,profile=profile,port=port,config=cfg,scopes=[row])
        manifest_path=CONTROL/'manifests'/f'{profile}.json'
        atomic_root(manifest_path,json.dumps(manifest).encode(),0o640,acc['gid'])
        env_path=CONTROL/'environment'/f'{profile}.env'
        # Systemd reads this fixed root-owned env, never a mutable profile env.
        token=next((line.split('=',1)[1] for line in (original/'.env').read_text().splitlines() if line.startswith('API_SERVER_KEY=')),None)
        if not token or not re.fullmatch('[a-f0-9]{64}',token):raise ValueError('Invalid private native capability')
        atomic_root(env_path,f'API_SERVER_KEY={token}\nAPI_SERVER_HOST=127.0.0.1\nAPI_SERVER_PORT={port}\n'.encode())
        library=pathlib.Path(tomllib.loads(xiar_paths.agent_control_config().read_text())['control']['sqlite_library'])
        if not re.fullmatch(r'/opt/xiar/sqlite/[0-9]+\.[0-9]+\.[0-9]+/libsqlite3\.so\.0',str(library)) or library.is_symlink() or library.stat().st_uid!=0 or library.stat().st_mode&0o022:raise ValueError('Unsafe external SQLite library')
        template=(pathlib.Path(__file__).parent/'xiar-agent.service.in').read_text()
        for key,value in {'@SQLITE@':str(library),'@USER@':acc['user'],'@AGENT@':agent,'@PROFILE@':profile,'@DATA@':str(home),'@PROJECT@':str(ROOT),'@NATIVE@':str(NATIVE),'@MANIFEST@':str(manifest_path),'@ENV@':str(env_path),'@BRAIN@':('LoadCredential=brain-tokens.json:'+str(pathlib.Path(OWNER.pw_dir)/'.config/xiar/brain-tokens.json') if agent=='xiar' else ''),'@GODSEYE_HISTORY@':f'BindReadOnlyPaths=-{ROOT}/data/godseye-history:/opt/xiar/project/data/godseye-history' if godseye_history_allowed(agent) else ''}.items():template=template.replace(key,value)
        if cfg.get('power')=='full':template=full_power_unit(template)
        path=pathlib.Path('/etc/systemd/system')/profile_unit(profile)
        if not path.exists() or path.read_text()!=template:atomic_root(path,template.encode(),0o644)
    sudo_grant(acc['user'],cfg.get('power')=='full' and dedicated)
    acc['hermes']=hermes_version()   # what this Agent's home and unit were rendered from; compared after a Hermes update
    all[agent]=acc;atomic_root(CONTROL/'accounts.json',json.dumps(all).encode())
    subprocess.run(['systemctl','daemon-reload'],check=True,capture_output=True,timeout=30)
    return {'dedicated':dedicated,'user':acc['user'],'profiles':len(scopes)}
def ensure_running(agent,profile,timeout):
    import cold
    cold.restore(agent,profile)   # an idle Agent whose data was sealed into cold storage gets it back before its service starts
    subprocess.run(['systemctl','start',profile_unit(profile)],check=True,capture_output=True,timeout=100)
    port=records()[agent]['profiles'][profile]['port'];deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        try:
            with urllib.request.urlopen(f'http://127.0.0.1:{port}/health',timeout=2) as response:
                if response.status==200:return port
        except Exception:pass
        time.sleep(.4)
    raise ValueError('Private Agent runtime did not reach startup readiness')
def api_key(agent,profile):
    path=Profiles(ROOT).home(slug(profile))/'.env'
    for line in path.read_text().splitlines():
        if line.startswith('API_SERVER_KEY='):return line.split('=',1)[1]
    raise ValueError('Private runtime capability unavailable')
