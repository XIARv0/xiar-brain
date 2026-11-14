# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Fixed-command root-owned systemd/cgroup worker units. No arbitrary account, exec, mount or credential path from callers."""
from pathlib import Path
import copy,json,os,pwd,re,secrets,shutil,stat,subprocess,tomllib
import runtime
import scheduler_workspace as workspace
import scheduler_memory as memory_proxy
ROOT=runtime.ROOT;OWNER=runtime.OWNER
ID=re.compile(r'^[a-f0-9]{32}$')
def unit(job):
 if not ID.fullmatch(job):raise ValueError('Invalid durable job id')
 return 'xiar-task-'+job+'.service'
def call(args,timeout=30):return subprocess.run(args,check=True,capture_output=True,text=True,timeout=timeout).stdout
def atomic(path,value,mode=0o600,gid=0):
 path=Path(path);path.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
 runtime.atomic_root(path,value if isinstance(value,bytes) else value.encode(),mode,gid)
def quoted(value):
 value=str(value)
 if any(c in value for c in ('\n','\r','\x00')):raise ValueError('Unsafe unit value')
 return '"'+value.replace('\\','\\\\').replace('"','\\"').replace('%','%%')+'"'
def definition(row,files,home,user,memory,template,cfg,library):
 job=row['id'];agent=template['_agent'];budget=int(template['budget_secs']);out=files/'output'
 lines=['[Unit]','Description=XIAR bounded subagent '+job,'PartOf=xiar-hermes-subagents.target']
 if row['parent']:lines.extend(['BindsTo='+unit(row['parent']),'After='+unit(row['parent'])])
 lines+=['[Service]','Type=exec','User='+user,'Group='+user,'Slice=xiar-'+row['weight']+'.slice','WorkingDirectory=/var/lib/xiar-agent/workspace','Environment=HOME=/var/lib/xiar-agent','Environment=HERMES_HOME=/var/lib/xiar-agent','Environment=XIAR_ROOT=/opt/xiar/project','Environment=XIAR_RUNTIME_MANIFEST=/run/xiar-task/manifest.json','Environment=XIAR_NATIVE_SOURCE=/opt/xiar/hermes','Environment=XIAR_EXECUTION_JOB='+job,'Environment=XIAR_TASK_INPUT=/run/xiar-task/task.json','Environment=XIAR_TASK_OUTPUT=/run/xiar-task/output','Environment=PYTHONPATH=/opt/xiar/agent-control/python:/opt/xiar/hermes:/opt/xiar/project/integrations/hermes-xiar/distribution','Environment=PYTHONDONTWRITEBYTECODE=1','Environment=LD_PRELOAD='+library,'Environment=XDG_RUNTIME_DIR=/var/lib/xiar-agent/run','LoadCredential=model-router-token:'+str(files/'model-router-token'),'TemporaryFileSystem=/run:ro','TemporaryFileSystem=/opt/xiar/project:ro','BindReadOnlyPaths='+quoted(str(ROOT/'config')+':/opt/xiar/project/config'),'BindReadOnlyPaths='+quoted(str(ROOT/'integrations/hermes-xiar')+':/opt/xiar/project/integrations/hermes-xiar'),'BindReadOnlyPaths='+quoted(str(ROOT/'Brain/Shared/mental-models')+':/opt/xiar/project/Brain/Shared/mental-models'),'BindReadOnlyPaths='+quoted(str(runtime.NATIVE)+':/opt/xiar/hermes'),'BindReadOnlyPaths='+quoted(str(files/'task.json')+':/run/xiar-task/task.json'),'BindReadOnlyPaths='+quoted(str(files/'manifest.json')+':/run/xiar-task/manifest.json'),'BindPaths='+quoted(str(home)+':/var/lib/xiar-agent'),'BindPaths='+quoted(str(out)+':/run/xiar-task/output'),'ExecStart=/opt/xiar/hermes/venv/bin/python -m xiar_hermes.worker','ProtectSystem=strict','ProtectHome=yes','PrivateTmp=yes','PrivatePIDs=yes','PrivateDevices=yes','NoNewPrivileges=yes','ProtectKernelTunables=yes','ProtectKernelModules=yes','ProtectControlGroups=yes','RestrictSUIDSGID=yes','CapabilityBoundingSet=','UMask=0077','Restart=no','KillMode=control-group','OOMPolicy=kill','SendSIGKILL=yes','FinalKillSignal=SIGKILL','RuntimeMaxSec='+str(budget),'TimeoutStopSec='+str(int(cfg['stop_grace_s'])),'MemoryMax='+str(int(cfg['worker_memory_max_mib']))+'M','MemorySwapMax=0','CPUQuota='+str(int(cfg['heavy_cpu_quota_percent'] if row['weight']=='heavy' else cfg['light_cpu_quota_percent']))+'%','StandardInput=null','StandardOutput=null','StandardError=null','InaccessiblePaths=-/opt/xiar/hermes/.env /mnt /media /srv']
 if agent=='xiar' and template.get('_brain_enabled',False):lines.append('LoadCredential=brain-tokens.json:'+OWNER.pw_dir+'/.config/xiar/brain-tokens.json')
 for rel in ('core','Brain/Templates'):
  lines.append('BindReadOnlyPaths='+quoted('-'+str(ROOT/rel)+':/opt/xiar/project/'+rel))
 if memory:lines.append('BindReadOnlyPaths='+quoted(str(memory)+':/var/lib/xiar-agent/memory-api-token'))
 return '\n'.join(lines)+'\n'

class Kernel:
 def __init__(self,cfg,store):self.cfg,self.store=cfg,store
 def files(self,job):unit(job);return self.store.base/'jobs'/job
 def state(self,job):
  q=subprocess.run(['systemctl','show',unit(job),'--property=LoadState,ActiveState,SubState,Result,MainPID,ControlGroup,ActiveEnterTimestampMonotonic'],capture_output=True,text=True,timeout=30)
  value=dict(line.split('=',1) for line in q.stdout.splitlines() if '=' in line)
  if q.returncode and value.get('LoadState')!='not-found':raise ValueError('Kernel worker status unavailable')
  return value
 def quiet(self,job):
  s=self.state(job)
  if s.get('ActiveState') not in ('inactive','failed') or s.get('MainPID','0')!='0':return False
  group=s.get('ControlGroup','')
  if group:
   p=(Path('/sys/fs/cgroup')/group.lstrip('/')).resolve()
   if not p.is_relative_to(Path('/sys/fs/cgroup')):return False
   events=p/'cgroup.events'
   if events.exists() and dict(line.split() for line in events.read_text().splitlines()).get('populated')!='0':return False
  return True
 def stop(self,job):
  subprocess.run(['systemctl','stop',unit(job)],capture_output=True,text=True,timeout=int(self.cfg()['stop_grace_s'])+30,check=False)
 def prepare(self,row,payload,template,placement):
  cfg=self.cfg();job=row['id'];files=self.files(job);files.mkdir(parents=True,exist_ok=True,mode=0o700)
  if not self.quiet(job):raise ValueError('Cannot prepare over a live worker')
  who=payload['principal'];profile=who['profile'];agent=who['agent'];adapter=memory_proxy.adapter(who)
  if profile=='owner':user=OWNER.pw_name;uid,gid=OWNER.pw_uid,OWNER.pw_gid
  else:
   scope=runtime.Profiles(ROOT).state()['scopes'].get(profile);account=runtime.records().get(agent)
   if not scope or not scope.get('enabled') or not account or profile not in account['profiles']:raise ValueError('Private runtime not provisioned/enabled')
   user=account['user'];uid,gid=account['uid'],account['gid']
  if uid==0 or user not in (OWNER.pw_name,runtime.username(agent)):raise ValueError('Unsafe worker account')
  # Fresh per-run HOME. Never bind the complete parent profile or its capability files.
  home=files/'home'
  if home.is_symlink():raise ValueError('Unsafe worker home')
  if home.exists():shutil.rmtree(home)
  home.mkdir(mode=0o700)
  for name in ('workspace','skills','memories','sessions','state','run'):(home/name).mkdir(mode=0o700)
  business=who.get('business','');enabled=bool(adapter['memory']['enabled'] and business)
  brain=bool(adapter['brain'].get('enabled') and agent=='xiar')
  adapter['identity']=dict(agent_id=agent,end_user_id=who['user'])
  adapter['profile']=dict(home='/var/lib/xiar-agent',read_only_roots=['/opt/xiar/project/integrations/hermes-xiar/skills','/opt/xiar/project/Brain/Shared/mental-models'])
  router=tomllib.loads((ROOT/'config/model_router.toml').read_text())['model_router']
  if router['host']!='127.0.0.1':raise ValueError('Worker proxy stays loopback')
  adapter['memory'].update(enabled=enabled,auto_recall=enabled,default_business=business,url='http://127.0.0.1:'+str(router['port'])+'/j/'+job+'/memory',token_file='/var/lib/xiar-agent/memory-api-token')
  adapter['routing'].update(businesses=[business] if business else [],state_dir='/var/lib/xiar-agent/state')
  if not enabled:
   for name in ('memory_routing','retrieval_strategy'):adapter['laya']['jobs'][name]=False
  adapter['brain'].update(enabled=brain,tokens_file='$CREDENTIALS_DIRECTORY/brain-tokens.json')
  adapter['laya'].update(url='http://127.0.0.1:'+str(router['port'])+'/j/'+job+'/laya',token_file='/var/lib/xiar-agent/memory-api-token')
  identity={k:who.get(k,'') for k in ('profile','agent','user','business')}
  delegation={k:payload['native'][k] for k in ('max_spawn_depth','orchestrator_enabled')}
  configs={'.xiar-identity.json':identity,'adapter.json':adapter,'config.yaml':dict(delegation=delegation,model=dict(provider='xiar-routed',default=placement['model']),plugins=dict(enabled=['xiar']),skills=dict(external_dirs=['/opt/xiar/project/integrations/hermes-xiar/skills']),memory=dict(memory_enabled=False,user_profile_enabled=False),terminal=dict(cwd='/var/lib/xiar-agent/workspace'))}
  for name,value in configs.items():atomic(home/name,json.dumps(value,allow_nan=False),0o640,gid)
  # A readable alias of ONLY the purpose job credential, never a full profile token.
  (home/'memory-api-token').symlink_to('/run/credentials/'+unit(job)+'/model-router-token')
  runtime.ownership(home,uid,gid)
  mapping=workspace.seed(home,payload,cfg,uid,gid,self.files(row['parent'])/'home/workspace' if row['parent'] else None)
  scope=dict(identity,memory_mode='xiar_memory' if enabled else 'none',enabled=True,active=True)
  manifest=dict(version=1,agent=agent,profile=profile,config=dict(id=agent,enabled=True,runtime_managed=True,memory_mode=scope['memory_mode']),scopes=[scope])
  task=copy.deepcopy(payload);task.update(core=memory_proxy.core(self.store.base,who) if adapter['memory']['enabled'] or memory_proxy.adapter(who)['memory']['enabled'] else '',template=template,run_id=job,placement=placement,workspace_mapping=mapping)
  atomic(files/'task.json',json.dumps(task,allow_nan=False),0o640,gid);atomic(files/'manifest.json',json.dumps(manifest),0o640,gid)
  if not (files/'model-router-token').exists():atomic(files/'model-router-token',secrets.token_urlsafe(48))
  out=files/'output';out.mkdir(exist_ok=True,mode=0o700);os.chown(out,uid,gid)
  library=tomllib.loads((ROOT/'config/agent_control.toml').read_text())['control']['sqlite_library'];library_path=Path(library)
  if not re.fullmatch(r'/opt/xiar/sqlite/[0-9]+\.[0-9]+\.[0-9]+/libsqlite3\.so\.0',library) or library_path.is_symlink() or library_path.stat().st_uid!=0 or library_path.stat().st_mode&0o022:raise ValueError('Unsafe patched SQLite runtime')
  if int(template['budget_secs'])<=0:raise ValueError('Nonpositive hard budget')
  text=definition(row,files,home,user,None,dict(template,_agent=agent,_brain_enabled=brain),cfg,library)
  atomic(Path('/etc/systemd/system')/unit(job),text,0o644)
  grammar=files/'xiar-worker-grammar.service';atomic(grammar,text.replace('ExecStart=/opt/xiar/hermes/venv/bin/python -m xiar_hermes.worker','ExecStart=/usr/bin/true'),0o644)
  call(['systemd-analyze','verify',str(grammar)]);call(['systemctl','daemon-reload'])
 def artifacts(self,row,payload):
  if not self.quiet(row['id']):raise ValueError('Worker must be quiet before artifact handoff')
  who=payload['principal'];account=runtime.records().get(who['agent']) if who['profile']!='owner' else None
  uid,gid=(account['uid'],account['gid']) if account else (OWNER.pw_uid,OWNER.pw_gid)
  parent=self.files(row['parent'])/'home/workspace' if row['parent'] else None
  result=workspace.export(self.files(row['id'])/'home',row['id'],who,self.cfg(),uid,gid,parent)
  if parent is not None:result['path']='/var/lib/xiar-agent/workspace/subagents/'+row['id']
  return result
 def start(self,job):call(['systemctl','start',unit(job)],timeout=60)
 def output(self,job,limit):
  p=self.files(job)/'output/result.json'
  if not p.exists():return None
  fd=os.open(p,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
  try:
   st=os.fstat(fd)
   if not stat.S_ISREG(st.st_mode) or st.st_size>limit:raise ValueError('Worker result outside bound')
   with os.fdopen(fd,'rb') as out:fd=None;value=json.loads(out.read(limit+1))
   if len(json.dumps(value,allow_nan=False).encode())>limit:raise ValueError('Encoded worker result exceeds transport bound')
   return value
  finally:
   if fd is not None:os.close(fd)
 def redact_spool(self,job):
  if not self.quiet(job):raise ValueError('Refusing cleanup of a live worker')
  for name in ('task.json','model-router-token','output/result.json','output/result.new'):
   p=self.files(job)/name
   if p.is_symlink():raise ValueError('Unsafe worker spool file')
   if p.exists():p.unlink()
