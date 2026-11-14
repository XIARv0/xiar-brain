# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Root-side health check of the private Agents, used after a Hermes update (Settings > Agents > Maintenance, and the xiar-agents-after-hermes-update skill).
Read-only: it never changes a file or a unit. The repair is `agents_resync` in server.py (re-render every Agent's home, unit and grant from the installed Hermes).
It reports facts and finding codes, never a token or any file content."""
import hashlib,json,os,pathlib,re,stat,subprocess
import paths as xiar_paths
import runtime
from xiar_hermes import profiles
from xiar_hermes.profiles import Profiles

HARDENING=('ProtectSystem=strict','ProtectHome=yes','NoNewPrivileges=yes','PrivateTmp=yes','PrivateDevices=yes','CapabilityBoundingSet=','ProtectKernelTunables=yes','AppArmorProfile=xiar-agent')

def anchored(parent):
    """Open a directory with every ancestor anchored by descriptor and symlinks refused (an Agent's home is written by the Agent's own user)."""
    path=pathlib.Path(parent)
    if not path.is_absolute() or any(part in ('.','..') for part in path.parts):raise ValueError('absolute path required')
    fd=os.open('/',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:
        for part in path.parts[1:]:
            child=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd);os.close(fd);fd=child
        return fd
    except BaseException:os.close(fd);raise

def read_small(path,limit=262144):
    """Anchored, no-follow, regular files only, bounded."""
    path=pathlib.Path(path);parent=anchored(path.parent)
    try:fd=os.open(path.name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=parent)
    finally:os.close(parent)
    try:
        info=os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_size>limit:raise ValueError('unsafe file')
        return os.read(fd,limit+1).decode('utf-8','replace'),info
    finally:os.close(fd)

def finding(code,severity,message,fix=''):return dict(code=code,severity=severity,message=message,fix=fix)

def check_agent(agent,cfg,acc,current,p,state,grants):
    out=[]
    recorded=acc.get('hermes')
    if not recorded:out.append(finding('not-synchronized','warning','This Agent has not been synchronized since Hermes versions were tracked.','resync'))
    elif recorded.get('fingerprint')!=current['fingerprint']:out.append(finding('hermes-changed','warning','Hermes changed from %s to %s since this Agent was last synchronized.'%(recorded.get('version'),current['version']),'resync'))
    if not acc.get('user') or acc.get('user')!=runtime.username(agent):out.append(finding('linux-user','critical','The Agent does not run as its own generated Linux user.','resync'))
    for row in (r for r in state['scopes'].values() if r['agent']==agent):
        profile=row['profile'];home=p.base/'isolated'/agent/profile
        try:
            adapter=json.loads(read_small(home/'adapter.json')[0])
            if agent!='xiar' and adapter.get('brain',{}).get('enabled'):out.append(finding('brain-on','critical','The neural Brain is switched on for a private Agent. It belongs only to Xiar.','resync'))
            if adapter.get('memory',{}).get('enabled') and not str(adapter['memory'].get('url','')).startswith('http://127.0.0.1:'):out.append(finding('memory-url','critical','The memory address is not loopback.','resync'))
        except (OSError,ValueError,KeyError):out.append(finding('adapter-unreadable','warning','The Agent configuration could not be read as expected.','resync'))
        try:
            config,_=read_small(home/'config.yaml')
            if '"xiar"' not in config and 'xiar' not in config:out.append(finding('plugin-missing','warning','The xiar plugin is not enabled in the Agent configuration.','resync'))
        except (OSError,ValueError):out.append(finding('config-unreadable','warning','The Agent Hermes configuration could not be read.','resync'))
        unit=pathlib.Path('/etc/systemd/system')/runtime.profile_unit(profile)
        try:
            text,_=read_small(unit)
            missing=[] if cfg.get('power')=='full' else [d for d in HARDENING if d not in text]
            if missing:out.append(finding('sandbox-weakened','critical','The Agent service is missing sandbox settings: '+', '.join(missing)+'.','resync'))
            if agent!='xiar' and 'brain-tokens' in text:out.append(finding('brain-credential','critical','The Agent service loads the Brain credential.','resync'))
            if ('User='+acc.get('user','?'))not in text:out.append(finding('unit-user','critical','The Agent service does not run as the Agent user.','resync'))
        except (OSError,ValueError):out.append(finding('unit-missing','warning','The Agent service definition is missing.','resync'))
        try:
            token,info=read_small(home/'memory-api-token',4096)
            if info.st_uid!=acc.get('uid') or info.st_mode&0o077:out.append(finding('token-permissions','critical','The Agent memory token is not private to the Agent user.','resync'))
            if cfg.get('memory_mode')=='xiar_memory':
                digest=hashlib.sha256(token.strip().encode()).hexdigest()
                if not any(g.get('token_sha256')==digest for g in grants):out.append(finding('grant-missing','warning','The Agent memory grant is not in the registry.','resync'))
        except (OSError,ValueError):
            if cfg.get('memory_mode')=='xiar_memory':out.append(finding('token-missing','warning','The Agent memory token is missing.','resync'))
        if row.get('active'):
            alive=subprocess.run(['systemctl','is-active',runtime.profile_unit(profile)],capture_output=True,text=True,timeout=15).stdout.strip()
            if alive!='active':out.append(finding('not-running','warning','The Agent is marked active but its service is %s.'%(alive or 'unknown'),'resync'))
    worst='critical' if any(f['severity']=='critical' for f in out) else 'attention' if out else 'ok'
    return dict(agent=agent,linux_user=acc.get('user',''),memory_mode=cfg.get('memory_mode'),status=worst,findings=out)

def check():
    current=runtime.hermes_version()
    p=Profiles(runtime.ROOT);state=p.state();records=runtime.records()
    try:grants=json.loads(read_small(xiar_paths.grants_path(),4*1024*1024)[0]).get('grants',[])
    except (OSError,ValueError):grants=[]
    agents=[check_agent(a,cfg,records.get(a,{}),current,p,state,grants) for a,cfg in state['agents'].items()]
    registry=check_registry_file()
    if registry:agents.append(dict(agent='(memory grants file)',linux_user='',memory_mode='',status='attention',findings=registry))
    return dict(hermes=current,agents=agents,ok=all(a['status']=='ok' for a in agents),checked=len(agents))

def check_registry_file():
    """The grants file must be root-owned, group xiar, mode 0640: no Agent user may read which grants exist."""
    import grp,os,stat
    path=xiar_paths.grants_path()
    try:info=os.lstat(path)
    except OSError:return [finding('grants-file-missing','warning','The memory grants file does not exist.','resync')]
    want=grp.getgrnam(profiles.GRANTS_GROUP).gr_gid
    if not stat.S_ISREG(info.st_mode) or info.st_uid!=0 or info.st_gid!=want or stat.S_IMODE(info.st_mode)!=profiles.GRANTS_MODE:
        return [finding('grants-file-mode','warning','The memory grants file is not root:%s mode %o (it is uid %d gid %d mode %o).'%(profiles.GRANTS_GROUP,profiles.GRANTS_MODE,info.st_uid,info.st_gid,stat.S_IMODE(info.st_mode)),'resync')]
    return []
