# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Durable scheduler, scoped status and kernel-confirmed release reconciliation. Root-owned broker module."""
from pathlib import Path
import copy,hmac,importlib.util,ipaddress,json,os,re,stat,threading,time,tomllib,urllib.request,urllib.parse
import runtime
import scheduler_memory as memory_proxy
from scheduler_upstream import Upstream
from scheduler_store import Store,ACTIVE,TERMINAL
from scheduler_kernel import Kernel,ID
ROOT=runtime.ROOT;OWNER=runtime.OWNER
SPEC_FIELDS={'max_iterations','max_tokens','enabled_toolsets','disabled_toolsets','ephemeral_system_prompt','prefill_messages','reasoning_config','session_id','parent_session_id'}
CONVERSATION_FIELDS={'user_message','system_message','conversation_history','task_id','persist_user_message','persist_user_timestamp','persist_user_display_kind','persist_user_display_metadata','persist_user_platform_id','turn_author'}
def owner_path(value):return Path(os.path.expandvars(str(value).replace('~',OWNER.pw_dir,1)))
def private_token(path,uid):
 p=Path(path);st=p.stat()
 if p.is_symlink() or not stat.S_ISREG(st.st_mode) or st.st_uid!=uid or st.st_mode&0o077:raise ValueError('Private scheduler capability unavailable')
 value=p.read_text().strip()
 if len(value)<32:raise ValueError('Invalid scheduler capability')
 return value
class Scheduler:
 def __init__(self):
  self.store=Store(self.cfg()['state_dir']);memory_proxy.service_key(self.store.base);self.kernel=Kernel(self.cfg,self.store);self.work_lock=threading.Lock();self.lifecycle=threading.RLock();self.upstream=Upstream(self)
  spec=importlib.util.spec_from_file_location('scheduler_template_controller',Path(__file__).with_name('scheduler_dispatch.py'));self.dispatch=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.dispatch)
 def cfg(self):
  cfg=tomllib.loads((ROOT/'config/scheduler.toml').read_text())['scheduler']
  for key in ('max_light','max_heavy_per_gpu','worker_memory_max_mib','max_queue','max_body_bytes','max_result_bytes','max_events','max_status_event_bytes','max_controls','max_steer_chars','stop_grace_s','workspace_max_files','workspace_max_bytes','workspace_file_max_bytes'):
   if isinstance(cfg[key],bool) or not isinstance(cfg[key],int) or cfg[key]<=0:raise ValueError('Invalid scheduler integer limit')
  if cfg['state_dir']!='/var/lib/xiar-scheduler' or len(set(cfg['priority_order']))!=len(cfg['priority_order']):raise ValueError('Unsafe scheduler storage/priority configuration')
  if cfg['max_result_bytes']+cfg['max_status_event_bytes']+cfg['max_controls']*cfg['max_steer_chars']*6>16773120 or cfg['max_status_event_bytes']<131072:raise ValueError('Scheduler response limit outside authenticated transport bounds')
  return cfg
 def principal(self,profile,authorization):
  if profile=='owner':
   cfg=tomllib.loads((ROOT/'config/model_router.toml').read_text())['model_router'];key=private_token(owner_path(cfg['token_file']),OWNER.pw_uid)
   adapter=tomllib.loads((ROOT/'config/hermes_adapter.toml').read_text());who=dict(profile='owner',agent=adapter['identity']['agent_id'],user=adapter['identity']['end_user_id'],business=adapter['memory']['default_business'])
  else:
   p=runtime.Profiles(ROOT);row=p.state()['scopes'].get(profile)
   if not row or not row.get('enabled'):raise ValueError('Private scheduler scope disabled')
   key=private_token(p.validate(row)/'model-router-token',OWNER.pw_uid);who={k:row[k] for k in ('profile','agent','user','business')}
  if not hmac.compare_digest(authorization.encode(),('Bearer '+key).encode()):raise ValueError('Scheduler capability refused')
  return who
 def job_principal(self,job,authorization):
  row=self.store.row(job)
  if not row or row['state'] not in ('launching','running','yielding','waiting_children','resuming','cancelling'):raise ValueError('Worker is not active')
  key=private_token(self.kernel.files(job)/'model-router-token',0)
  if not hmac.compare_digest(authorization.encode(),('Bearer '+key).encode()):raise ValueError('Worker capability refused')
  payload=self.store.open(job,'payload',row['payload']);who=copy.deepcopy(payload['principal'])
  if who['profile']!='owner':
   current=runtime.state()['scopes'].get(who['profile'])
   if not current or not current.get('enabled') or any(current[k]!=who[k] for k in ('agent','user','business')):raise ValueError('Worker scope revoked')
  if not self.live(row):raise ValueError('Worker cgroup is not running')
  who['run_id']=job;return who
 def laya(self,path,body=None):
  rt=tomllib.loads((ROOT/'config/llm_runtime.toml').read_text());a=tomllib.loads((ROOT/'config/model_router.toml').read_text())['model_router'];base='http://'+rt['laya'].get('serve_bind','127.0.0.1:8772')
  if body is None:req=urllib.request.Request(base+path)
  else:
   common=private_token(owner_path(rt['laya'].get('serve_token_file','~/.config/xiar/laya-token')),OWNER.pw_uid);execution=private_token(owner_path(a['execution_token_file']),OWNER.pw_uid)
   req=urllib.request.Request(base+path,data=json.dumps(body,allow_nan=False).encode(),headers={'Content-Type':'application/json','Authorization':'Bearer '+common,'X-Xiar-Execution-Key':execution},method='POST')
  with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req,timeout=a['laya_timeout_s']) as q:return json.loads(q.read(self.cfg()['max_body_bytes']))
 def emergency(self):
  if hasattr(self,'safety'):return self.safety.current()
  from autonomy import status
  return status()
 def live(self,row):return self.kernel.state(row['id']).get('ActiveState') in ('active','activating','deactivating')
 def descendants(self,job):
  rows=self.store.rows();ids={job};changed=True
  while changed:
   changed=False
   for row in rows:
    if row['parent'] in ids and row['id'] not in ids:ids.add(row['id']);changed=True
  return [row for row in rows if row['id'] in ids]
 def access(self,row,who):
  if not row or row['scope']!=who['profile']:raise ValueError('Foreign scheduler scope refused')
  if who.get('run_id') and row['id'] not in {v['id'] for v in self.descendants(who['run_id'])}:raise ValueError('Foreign worker lineage refused')
 def status(self,row,after=0):
  value={k:row[k] for k in ('id','state','reason','template','weight','released')};value['run_id']=row['id'];value['controls']=self.store.control_summary(row['id']);value['archive']=dict(episodic=row['archived']==1,deferred=row['archived']==-1);value['events']=self.store.events(row['id'],int(after),self.cfg()['max_status_event_bytes'])
  if row['state'] in TERMINAL and row['result'] is not None:value['output']=self.store.open(row['id'],'result',row['result'])
  return value
 def cancel(self,row):
  with self.lifecycle:
   for child in reversed(self.descendants(row['id'])):
    child=self.store.row(child['id'])
    if child['state'] in TERMINAL:continue
    queued=child['state']=='queued'
    self.store.update(child['id'],state='cancelled' if queued else 'cancelling',reason='Explicit cancellation requested',released=1 if queued else 0)
    if queued:
     controls=self.store.controls(child['id'],0,close=True);pending='\n'.join(v['text'] for v in controls if not v['delivered'])
     self.store.update(child['id'],result={'result':{'completed':False,'interrupted':True,'error':'Cancelled before execution','pending_steer':pending},'metrics':{},'accepted_controls':[v['id'] for v in controls]})
    else:self.kernel.stop(child['id'])
    self.store.audit(child['id'],'cancel','Cancellation requested; permits retained until cgroup quiet')
 def request(self,body,who):
  with self.lifecycle:return self._request(body,who)
 def _request(self,body,who):
  cfg=self.cfg();action=body.get('action');job=str(body.get('run_id',''))
  if action=='submit':
   if self.emergency()['stopped']:raise ValueError('Emergency stop: durable admission closed')
   if not cfg['enabled']:raise ValueError('Durable worker binding staged, not activated')
   if not ID.fullmatch(job):raise ValueError('Client must retain a valid idempotent run id')
   brief=body.get('brief');spec=body.get('spec',{});conversation=body.get('conversation',{})
   if not isinstance(brief,str) or not brief.strip() or not isinstance(spec,dict) or set(spec)-SPEC_FIELDS or not isinstance(conversation,dict) or set(conversation)-CONVERSATION_FIELDS:raise ValueError('Unknown native worker contract')
   priority=body.get('priority','normal')
   if priority not in cfg['priority_order']:raise ValueError('Unknown queue priority')
   if who.get('run_id'):
    parent=self.store.row(who['run_id'])
    if parent is None or parent['state'] not in ('launching','running','yielding','waiting_children'):raise ValueError('Parent worker not running')
   who=copy.deepcopy(who)
   business=str(body.get('business') or who.get('business') or '')
   if business:
    runtime.slug(business)
    if who.get('run_id') or who['profile']!='owner':
     if business!=who.get('business'):raise ValueError('Worker business cannot change')
    else:
     adapter=memory_proxy.adapter(who);allowed=set(adapter['routing'].get('businesses',[]))|{adapter['memory'].get('default_business','')}
     if business not in allowed:raise ValueError('Choose an explicitly configured business')
   who['business']=business
   native=body.get('native',{})
   if not isinstance(native,dict) or native.get('context') is not None and not isinstance(native.get('context'),str):raise ValueError('Invalid explicit native task context')
   if who.get('run_id'):
    inherited=self.store.open(parent['id'],'payload',parent['payload'])['native'];maximum=inherited['max_spawn_depth'];orchestrator=inherited['orchestrator_enabled'];depth=inherited['depth']+1
    if depth>maximum or not orchestrator:raise ValueError('Native delegation depth/kill switch refused')
   else:
    maximum=native.get('max_spawn_depth',1);orchestrator=native.get('orchestrator_enabled',True);depth=1
    if isinstance(maximum,bool) or not isinstance(maximum,int) or maximum<1 or not isinstance(orchestrator,bool):raise ValueError('Invalid native configured depth/kill switch')
   native=dict(max_spawn_depth=maximum,orchestrator_enabled=orchestrator,depth=depth,role='orchestrator' if orchestrator and depth<maximum else 'leaf',context=native.get('context'),subagent_id=str(native.get('subagent_id',''))[:128])
   # The architecture grants task/explicit context + allowed Core/business recall,
   # never automatic unscoped history inherited from the parent's full session.
   spec=copy.deepcopy(spec);conversation=copy.deepcopy(conversation)
   for key in ('prefill_messages','ephemeral_system_prompt'):spec.pop(key,None)
   for key in ('conversation_history','system_message'):conversation.pop(key,None)
   payload=dict(principal={k:who.get(k,'') for k in ('profile','agent','user','business')},brief=brief,template_id=body.get('template_id',''),spec=spec,conversation=conversation,priority=priority,native=native,workspace=str(body.get('workspace') or ''),memory_enabled=bool(memory_proxy.adapter(who)['memory']['enabled'] and business))
   if len(json.dumps(payload,allow_nan=False).encode())>cfg['max_body_bytes']:raise ValueError('Task exceeds configured bound')
   self.store.submit(job,who['profile'],who.get('run_id',''),cfg['priority_order'].index(priority),payload,cfg['max_queue'])
   intent='cancel_intent:'+who['profile']+':'+who.get('run_id','')+':'+job
   with self.store.db() as db:cancelled=db.execute('SELECT value FROM state WHERE key=?',(intent,)).fetchone() is not None
   if cancelled:
    self.cancel(self.store.row(job))
    with self.store.db() as db:db.execute('DELETE FROM state WHERE key=?',(intent,))
   if who.get('run_id'):
    parent=self.store.row(who['run_id'])
    if self.store.row(job)['state'] not in TERMINAL and parent['weight']=='heavy' and parent['state'] in ('launching','running'):self.store.update(parent['id'],state='yielding',reason='Waiting for scoped child; model turn handoff requested')
   self.store.audit(job,'queued','Encrypted durable queue accepted');self.record(self.store.row(job),'queued');return self.status(self.store.row(job))
  if action in ('list_running','queue_status'):
   rows=self.store.rows(scope=who['profile'])
   if who.get('run_id'):rows=[v for v in rows if v['id'] in {d['id'] for d in self.descendants(who['run_id'])}]
   return {'jobs':[self.status(v) for v in rows if v['state'] not in TERMINAL]}
  row=self.store.row(job)
  if action=='cancel' and row is None:
   if not ID.fullmatch(job):raise ValueError('Invalid cancellation intent id')
   key='cancel_intent:'+who['profile']+':'+who.get('run_id','')+':'+job
   with self.store.db() as db:
    if not db.execute('SELECT value FROM state WHERE key=?',(key,)).fetchone() and db.execute("SELECT count(*) FROM state WHERE key LIKE 'cancel_intent:%'").fetchone()[0]>=cfg['max_queue']:raise ValueError('Cancellation intent bound reached')
    db.execute('INSERT OR IGNORE INTO state(key,value) VALUES(?,?)',(key,'1'))
   return dict(run_id=job,state='cancelled',output={'result':{'completed':False,'interrupted':True,'error':'Cancellation intent durable before submission'},'metrics':{}})
  self.access(row,who)
  if action=='status':return self.status(row,body.get('cursor',0))
  if action=='cancel':self.cancel(row);return self.status(self.store.row(job))
  if action=='lease':
   if row['state'] not in ('launching','running','yielding','waiting_children','resuming') or row['placement'] is None or not self.live(row):return {'active':False}
   p=self.store.open(job,'placement',row['placement']);return {'active':True,'connection':p['id'],'model':p['model'],'scope':row['scope']}
  if action=='steer':
   control=str(body.get('control_id',''));text=body.get('text')
   if not ID.fullmatch(control) or not isinstance(text,str) or not text.strip() or len(text)>cfg['max_steer_chars']:raise ValueError('Steering payload outside bound')
   accepted=self.store.steer(job,control,text.strip(),cfg['max_controls']);return dict(accepted=accepted,control_id=control)
  if action in ('control_pull','control_close'):
   if who.get('run_id')!=job:raise ValueError('Only the actual worker may consume controls')
   delivered=body.get('delivered',[])
   if not isinstance(delivered,list) or len(delivered)>cfg['max_controls'] or any(not isinstance(v,str) or not ID.fullmatch(v) for v in delivered):raise ValueError('Invalid delivered control receipt')
   return dict(controls=self.store.controls(job,body.get('cursor',0),delivered,action=='control_close'))
  if action in ('laya_decide','laya_health'):
   if who.get('run_id')!=job or row['state'] not in ('launching','running'):raise ValueError('Only running workers may use advisory Laya')
   return self.laya('/health') if action=='laya_health' else self.laya('/v1/decide',body.get('body',{}))
  if action=='memory':
   if who.get('run_id')!=job or row['state'] not in ('launching','running'):raise ValueError('Only a running scoped worker may use memory')
   code,value=memory_proxy.proxy(who,str(body.get('path','')),body.get('body',{}),body.get('headers',{}));return dict(status=code,body=value)
  if action=='event':
   if who.get('run_id')!=job or not isinstance(body.get('event'),dict):raise ValueError('Only live worker may publish its events')
   if len(json.dumps(body['event']).encode())>65536:raise ValueError('Worker event exceeds bound')
   self.store.event(job,body['event'],cfg['max_events']);return {'recorded':True}
  raise ValueError('Unknown durable scheduler action')
 def connections(self):
  cfg=tomllib.loads((ROOT/'config/model_router.toml').read_text())['model_router'];key=private_token(owner_path(cfg['token_file']),OWNER.pw_uid)
  req=urllib.request.Request('http://'+cfg['host']+':'+str(cfg['port'])+'/v1/connections',headers={'Authorization':'Bearer '+key})
  with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req,timeout=cfg['laya_timeout_s']) as q:rows=json.loads(q.read(self.cfg()['max_body_bytes']))['connections']
  out=[];settings=tomllib.loads((ROOT/'config/subagent_dispatch.toml').read_text())['dispatch']
  for c in rows:
   url=str(c.get('base_url') or 'http://'+str(c.get('host',''))+':'+str(c.get('port','')));host=urllib.parse.urlsplit(url).hostname or ''
   try:local=ipaddress.ip_address(host).is_loopback
   except ValueError:local=host in ('localhost','')
   device=c.get('device','auto') if local else 'cpu'
   if device not in ('cpu','gpu','npu'):continue
   if not local:memory=settings['remote_control_mib']
   elif isinstance(c.get('dispatch_memory_mib'),int):memory=c['dispatch_memory_mib']
   else:
    path=Path(str(c.get('model_path',''))).expanduser()
    if not path.is_file():continue
    memory=(path.stat().st_size+1048575)//1048576+settings['placement_overhead_mib']
   out.append(dict(id=c['id'],model=c['served_name'],device=device,gpu=int(c.get('gpu',0)),engine=str(c.get('runner','http')),memory_mib=int(memory),note=str(c.get('description','Configured chat connection'))[:200]))
  return out
 def gate(self,row,template,placements,sample):
  cfg=self.cfg();running=[v for v in self.store.rows(ACTIVE) if v['id']!=row['id'] and v['state']!='waiting_children']
  if sample['ram_available_mib']<cfg['ram_floor_mib']+cfg['worker_memory_max_mib']:return [],'RAM floor and bounded worker allocation require waiting'
  if row['priority']!=0 and sample['cpu_percent']>=cfg['shed_cpu_percent']:return [],'CPU soft ceiling requires waiting'
  if template.get('weight','heavy')=='light':
   if sum(v['weight']=='light' for v in running)>=cfg['max_light']:return [],'Configured light pool full'
   return placements,'Light pool capacity available'
  result=[]
  for p in placements:
   busy=0
   for old in running:
    if old['weight']!='heavy' or old['placement'] is None:continue
    op=self.store.open(old['id'],'placement',old['placement'])
    if op['device']==p['device'] and op.get('gpu',0)==p.get('gpu',0):busy+=1
   if busy>=cfg['max_heavy_per_gpu']:continue
   if p['device']=='gpu':
    gpu=next((v for v in sample['gpus'] if v['index']==p['gpu']),None)
    if not gpu or gpu['vram_total_mib']-gpu['vram_used_mib']<int(template['vram_reserve_mib']):continue
   result.append(p)
  return result,'Heavy device/VRAM slot available' if result else 'Heavy device slot or template VRAM reserve requires waiting'
 def record(self,row,state):
  payload=self.store.open(row['id'],'payload',row['payload'])
  return memory_proxy.lifecycle(self.store.base,row['id'],payload['principal'],state,row['template'],payload.get('memory_enabled',False))
 def archive(self):
  for row in self.store.rows(TERMINAL):
   if row['archived']!=0:continue
   response=self.record(row,row['state'])
   if response.get('audited'):self.store.update(row['id'],archived=1 if response.get('archived') else -1)
 def release(self,row):
  if not self.kernel.quiet(row['id']) or not self.upstream.quiet(row['id']):return False
  if row['released']:return True
  result=self.laya('/v1/release',{'run_id':row['id'],'scope':row['scope']})
  if result.get('released') or result.get('active') is False:self.store.update(row['id'],released=1);return True
  return False
 def reconcile(self):
  with self.lifecycle:return self._reconcile()
 def _reconcile(self):
  for row in reversed(self.store.rows(ACTIVE)):
   job=row['id'];payload=self.store.open(job,'payload',row['payload']);who=payload['principal']
   if who['profile']!='owner':
    current=runtime.state()['scopes'].get(who['profile'])
    if not current or not current.get('enabled') or any(current[k]!=who[k] for k in ('agent','user','business')):
     if row['state']!='cancelling':self.cancel(row);row=self.store.row(job)
   state=self.kernel.state(job)
   if state.get('ActiveState') in ('active','activating'):
    manifest=ROOT/'Brain/Templates'/row['template']/'manifest.json'
    if manifest.is_file():
     template=json.loads(manifest.read_text());warning=template.get('soft_warn_secs');start=int(state.get('ActiveEnterTimestampMonotonic','0') or 0)/1000000
     if isinstance(warning,(int,float)) and not isinstance(warning,bool) and 0<warning<int(template['budget_secs']) and start>0 and time.monotonic()-start>=warning:
      with self.store.db() as db:marked=db.execute('INSERT OR IGNORE INTO state(key,value) VALUES(?,?)',('soft_warning:'+job,'1')).rowcount
      if marked:self.store.event(job,dict(kind='budget_warning',value=dict(reason='Configured soft budget reached; hard kernel budget remains active',soft_warn_secs=warning)),self.cfg()['max_events'])
    if row['state']=='launching':self.store.update(job,state='running',reason='Kernel-bounded worker running')
    continue
   if not self.kernel.quiet(job):continue
   descendants=[v for v in self.descendants(job) if v['id']!=job]
   outstanding=[v for v in descendants if v['state'] not in TERMINAL]
   for child in outstanding:
    if child['state']!='cancelling':self.cancel(child)
   if outstanding:continue # children export first; parent handoff must include their quiescent artifacts
   if row['state']=='admitting' or row['state']=='launching' and state.get('LoadState')=='not-found':
    if self.release(row):self.store.update(job,state='queued',placement=None,generation='',reason='Interrupted admission safely reconciled')
    continue
   try:output=self.store.open(job,'result',row['result']) if row['state']=='releasing' and row['result'] is not None else self.kernel.output(job,self.cfg()['max_result_bytes'])
   except (OSError,ValueError,RecursionError):output=None
   terminal=row['reason'] if row['state']=='releasing' and row['reason'] in TERMINAL else 'cancelled' if row['state']=='cancelling' else 'killed_budget' if state.get('Result') in ('timeout','oom-kill') else 'completed' if state.get('Result')=='success' and output is not None else 'failed'
   if not isinstance(output,dict) or not isinstance(output.get('result'),dict):output={'result':{'completed':False,'failed':True,'error':'No valid native worker result'}};terminal='failed' if terminal=='completed' else terminal
   if terminal=='completed' and (output['result'].get('failed') or output['result'].get('error')):terminal='failed'
   if terminal!='completed':output['result'].update(completed=False,failed=True,interrupted=terminal in ('cancelled','killed_budget'),error='Kernel worker ended: '+terminal)
   if row['state']!='releasing':
    delivered=output.get('delivered_controls',[])
    if not isinstance(delivered,list):delivered=[]
    controls=self.store.controls(job,0,delivered,True);output['accepted_controls']=[v['id'] for v in controls]
    unconfirmed=[v['text'] for v in controls if not v['delivered']]
    if unconfirmed:output['result']['pending_steer']='\n'.join([output['result'].get('pending_steer',''),*unconfirmed]).strip()
    payload=self.store.open(job,'payload',row['payload'])
    try:output['workspace_handoff']=self.kernel.artifacts(row,payload)
    except (OSError,ValueError,RecursionError):output['workspace_handoff']={'error':'Workspace handoff refused; private workspace retained for recovery'}
   self.store.update(job,state='releasing',result=output,reason=terminal)
   # Cancelling a parent must also stop descendants (BindsTo enforces this even with the broker down).
   for child in self.descendants(job):
    if child['id']!=job and child['state'] not in TERMINAL:self.cancel(child)
   fresh=self.store.row(job)
   if self.release(fresh):
    self.kernel.redact_spool(job);self.store.update(job,state=terminal,reason='Kernel cgroup quiet; '+terminal);self.store.audit(job,terminal,'Confirmed cgroup empty; native permit reconciled')
 def handoffs(self):
  with self.lifecycle:return self._handoffs()
 def _handoffs(self):
  # Live parent processes retain their own cgroup budget but relinquish the heavy
  # model permit only after every sender is closed/proven idle. New parent model
  # requests wait at the durable transport boundary until its children finish.
  for row in self.store.rows(('yielding','waiting_children','resuming')):
   if not self.live(row):continue
   children=[v for v in self.descendants(row['id']) if v['id']!=row['id'] and v['state'] not in TERMINAL]
   if row['state']=='yielding':
    if not self.upstream.quiet(row['id']):continue
    result=self.laya('/v1/release',{'run_id':row['id'],'scope':row['scope']})
    if not (result.get('released') or result.get('active') is False):continue
    self.store.update(row['id'],state='waiting_children',released=1,reason='Parent inference drained; heavy permit handed to scoped children');self.store.audit(row['id'],'yield','Upstream drained; heavy parent permit released for children')
    row=self.store.row(row['id'])
   if children:continue
   self.store.update(row['id'],state='resuming',reason='Children ended; reacquiring parent model permit')
   payload=self.store.open(row['id'],'payload',row['payload']);placement=self.store.open(row['id'],'placement',row['placement']);controller=self.dispatch.Controller(ROOT,self.laya,None)
   template,why,confidence=controller.select(payload['brief'],row['template'])
   if not template:continue
   sample=self.laya('/v1/resources',{})['sample'];eligible,reason=self.gate(row,template,[placement],sample)
   if not eligible:continue
   generation=self.laya('/health').get('instance_id')
   if not generation:continue
   decision=self.laya('/v1/delegate',{'run_id':row['id'],'scope':row['scope'],'text':payload['brief'][:1500],'placements':[placement],'priority':self.cfg()['native_priority'][payload['priority']]})
   if decision.get('status')!='run':continue
   self.store.update(row['id'],state='running',released=0,generation=generation,reason='Children complete; parent model permit reacquired');self.store.audit(row['id'],'resume','Scoped parent model permit reacquired after children')
 def tick(self):
  if not self.work_lock.acquire(blocking=False):return
  try:
   self.reconcile();self.handoffs();self.archive()
   if not self.cfg()['enabled'] or self.emergency()['stopped']:return
   generation=self.laya('/health').get('instance_id')
   if not generation:raise ValueError('Laya execution epoch missing; admission closed')
   pending_epoch=False
   with self.lifecycle:
    for old in self.store.rows(ACTIVE):
     if not old['generation'] or old['generation']==generation:continue
     if old['state'] in ('cancelling','releasing','admitting','launching') or not self.live(old) or not self.upstream.quiet(old['id']):pending_epoch=True;continue
     # A restarted native governor lost its volatile lease, not our durable
     # worker. Block new inference and reacquire before it can use the model.
     self.store.update(old['id'],state='waiting_children',released=1,generation=generation,reason='Native governor restarted; quiescent worker awaits lease reacquisition')
     self.store.audit(old['id'],'epoch_reconcile','Quiescent worker retained; model use blocked until reacquisition')
   if pending_epoch:return
   self.handoffs()
   for row in self.store.rows(('queued',)):
    if row['updated']+self.cfg()['failure_backoff_s']>time.time() and row['reason']!='Durably queued':continue
    if row['parent']:
     parent=self.store.row(row['parent'])
     if parent and parent['weight']=='heavy' and parent['state'] not in ('waiting_children',):continue
    payload=self.store.open(row['id'],'payload',row['payload']);self.record(row,'queued');controller=self.dispatch.Controller(ROOT,self.laya,None)
    template,why,confidence=controller.select(payload['brief'],payload['template_id'])
    if not template:self.store.update(row['id'],reason=why);continue
    self.store.event(row['id'],dict(kind='template_decision',value=dict(template=template['id'],confidence=confidence,reason=why)),self.cfg()['max_events'])
    template=copy.deepcopy(template);template['context']='\n\n'.join((ROOT/'Brain/Templates'/template['id']/name).read_text() for name in ('SOUL.md','AGENTS.md','SKILLS.md'))
    sample=self.laya('/v1/resources',{})['sample'];placements,why=self.gate(row,template,self.connections(),sample)
    if not placements:self.store.update(row['id'],reason=why);continue
    if not self.store.transition(row['id'],('queued',),state='admitting',released=0,template=template['id'],weight=template.get('weight','heavy'),generation=generation,reason='Trusted native resource admission'):continue
    native=tomllib.loads((ROOT/'config/scheduler.toml').read_text())['scheduler']['native_priority'][payload['priority']]
    decision=self.laya('/v1/delegate',{'run_id':row['id'],'scope':row['scope'],'text':payload['brief'][:1500],'placements':placements,'priority':native})
    self.store.event(row['id'],dict(kind='resource_decision',value={k:decision[k] for k in ('status','reason','reasons','resource_confidence','placement') if k in decision}),self.cfg()['max_events'])
    if decision.get('status')!='run':
     self.store.transition(row['id'],('admitting',),state='queued',reason=str(decision.get('reason') or decision.get('reasons') or 'Native gate waits')[:300]);continue
    selected=next(v for v in placements if v['id']==decision['placement']['id'])
    with self.lifecycle:
     if self.store.row(row['id'])['state']!='admitting':
      self.release(self.store.row(row['id']));continue
     self.store.update(row['id'],state='launching',placement=selected,released=0,reason='Permit durable; preparing kernel worker')
     current=self.store.row(row['id']);self.record(current,'launching');self.kernel.prepare(current,payload,template,selected)
     if self.emergency()['stopped']:self.cancel(current);continue
     self.kernel.start(row['id']);self.store.update(row['id'],state='running',reason='Kernel RuntimeMaxSec and memory cgroup active');self.store.audit(row['id'],'running','Laya choice, durable reservation and kernel unit installed');self.record(self.store.row(row['id']),'running')

  finally:self.work_lock.release()
 def start(self):
  def loop():
   while True:
    try:self.tick()
    except Exception as e:self.store.audit('scheduler','reconcile_retry',type(e).__name__)
    threading.Event().wait(float(self.cfg()['tick_s']))
  threading.Thread(target=loop,daemon=True,name='xiar-scheduler-reconcile').start()
