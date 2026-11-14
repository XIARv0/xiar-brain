# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""External native context + actual-child resource lifecycle. No upstream file writes."""
from pathlib import Path
import contextvars,functools,inspect,json,os,threading,time,tomllib,urllib.error,urllib.request
CURRENT=contextvars.ContextVar('xiar_execution_agent',default=None)
LEASE=contextvars.ContextVar('xiar_execution_lease',default=None)
TEMPLATE=contextvars.ContextVar('xiar_requested_template',default='')
BUSINESS=contextvars.ContextVar('xiar_requested_business',default='')
PRIORITY=contextvars.ContextVar('xiar_requested_priority',default='normal')
_INSTALLED=False

def root():return Path(os.environ.get('XIAR_ROOT',Path(__file__).resolve().parents[3]))
def enabled():
 a=tomllib.loads((root()/'config/hermes_adapter.toml').read_text())
 return bool(a.get('laya',{}).get('enabled',True) and a.get('laya',{}).get('jobs',{}).get('subagent_dispatch',False))
def client(agent,body,path='/dispatch',timeout=30):
 from .model_overlay import binding
 b=binding(root(),Path(agent._xiar_execution_home))
 if b is None:raise ValueError('Authenticated model transport required for dispatch')
 import shlex
 args=shlex.split(b['provider']['key_cmd']);credential_path=Path(args[args.index('--file')+1]);s=credential_path.stat()
 if credential_path.is_symlink() or s.st_uid!=os.getuid() or s.st_mode&0o077:raise ValueError('Invalid private execution credential')
 token=credential_path.read_text().strip();req=urllib.request.Request(b['provider']['api']+path,data=json.dumps(body).encode(),headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'},method='POST')
 with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req,timeout=timeout) as q:
  raw=q.read(16777217)
  if len(raw)>16777216:raise ValueError('Authenticated execution response exceeds bound')
  return json.loads(raw)
def contracts():
 from run_agent import AIAgent
 from tools.delegate_tool_dispatch import _Batch
 from agent.agent_runtime_helpers import invoke_tool
 required={'agent','function_name','function_args'}
 if not required.issubset(inspect.signature(invoke_tool).parameters):raise RuntimeError('Native invocation contract changed')
 if not hasattr(AIAgent,'_invoke_tool') or not hasattr(AIAgent,'_execute_tool_calls_sequential') or not hasattr(AIAgent,'_dispatch_delegate_task'):raise RuntimeError('Native execution entry changed')
 if list(inspect.signature(_Batch.run_child).parameters)!=['self','i','task','child']:raise RuntimeError('Native child lifetime contract changed')
 return {'context':True,'actual_child_lifetime':True,'acceptance_run':False}
def install():
 global _INSTALLED
 if _INSTALLED:return
 contracts()
 from run_agent import AIAgent
 from tools.delegate_tool_dispatch import _Batch
 original_init=AIAgent.__init__
 @functools.wraps(original_init)
 def initialize(self,*a,**kw):
  original_init(self,*a,**kw)
  from hermes_constants import get_hermes_home
  self._xiar_execution_home=str(get_hermes_home())
  self._xiar_requested_template=TEMPLATE.get()
  self._xiar_requested_business=BUSINESS.get();self._xiar_requested_priority=PRIORITY.get()
  from .worker import SPEC_FIELDS
  self._xiar_worker_spec={k:v for k,v in kw.items() if k in SPEC_FIELDS}
  for name in ('session_id','parent_session_id'):
   value=getattr(self,name,None)
   if value is not None:self._xiar_worker_spec[name]=value
 AIAgent.__init__=initialize
 for name in ('_invoke_tool','_execute_tool_calls_sequential','_dispatch_delegate_task'):
  original=getattr(AIAgent,name)
  def make(fn):
   @functools.wraps(fn)
   def scoped(self,*a,**kw):
    token=CURRENT.set(self)
    try:return fn(self,*a,**kw)
    finally:CURRENT.reset(token)
   return scoped
  setattr(AIAgent,name,make(original))
 original_child=_Batch.run_child
 @functools.wraps(original_child)
 def child_lifetime(batch,i,task,child):
  from .process_execution import configured,run as process_run
  scheduler=tomllib.loads((root()/'config/scheduler.toml').read_text())['scheduler']
  if configured(root()):
   if not enabled():return {'task_index':i,'error':'Configured dispatch disabled; uncontrolled native fallback forbidden'}
   return process_run(original_child,batch,i,task,child)
  if not scheduler.get('legacy_native_fallback',False):return {'task_index':i,'error':'Durable execution disabled; legacy timer fallback forbidden'}
  if not enabled():return original_child(batch,i,task,child)
  parent=batch.parent_agent
  if not getattr(parent,'_xiar_execution_home',None):return {'task_index':i,'error':'XIAR parent scope unavailable; no fallback'}
  cfg=tomllib.loads((root()/'config/subagent_dispatch.toml').read_text())['dispatch'];result=None
  while not getattr(child,'_interrupt_requested',False):
   try:result=client(parent,{'action':'acquire','brief':task['goal'],'template_id':getattr(child,'_xiar_requested_template','')})
   except (OSError,ValueError,urllib.error.URLError):result={'status':'wait','reason':'Authenticated execution controller unavailable'}
   if result.get('status')=='run':break
   if result.get('status')=='never':return {'task_index':i,'error':'XIAR placement cannot fit configured hardware'}
   time.sleep(float(cfg['queue_poll_s']))
  if not result or result.get('status')!='run':return {'task_index':i,'error':'XIAR queued child cancelled','interrupted':True}
  run=result['run_id'];template=result['template'];timer=None;token=LEASE.set(run)
  original_client=getattr(child,'client',None)
  try:
   if original_client is None or not hasattr(original_client,'with_options'):raise ValueError('Native child client pinning contract changed')
   child.client=original_client.with_options(default_headers={**original_client.default_headers,'X-Xiar-Dispatch-Run':run})
   def budget_stop():
    child.hard_interrupt('XIAR configured subagent budget reached',tool_reason='xiar_budget')
   timer=threading.Timer(int(template['budget_secs']),budget_stop);timer.daemon=True;timer.start()
   enriched=dict(task,goal='[Configured subagent template '+template['id']+']\n'+template['context']+'\n[Assigned task]\n'+task['goal'])
   return original_child(batch,i,enriched,child)
  except Exception:return {'task_index':i,'error':'XIAR child execution contract refused; no uncontrolled dispatch'}
  finally:
   if timer:timer.cancel()
   LEASE.reset(token)
   # Only ACTUAL child return/confirmed interruption reaches here, never background-handle return.
   try:client(parent,{'action':'release','run_id':run})
   except (OSError,ValueError,urllib.error.URLError):
    # Retain the server permit on failure: fail closed rather than silently overcommit.
    pass
 _Batch.run_child=child_lifetime;_INSTALLED=True

def dispatch(args):
 agent=CURRENT.get()
 if not enabled() or not _INSTALLED or agent is None:raise ValueError('Native authenticated dispatch context unavailable')
 if not isinstance(args,dict) or not isinstance(args.get('brief'),str):raise ValueError('Task brief required')
 token=TEMPLATE.set(str(args.get('template_id') or ''));business=BUSINESS.set(str(args.get('business') or ''));priority=PRIORITY.set(str(args.get('priority') or 'normal'))
 try:return agent._dispatch_delegate_task({'goal':args['brief'],'context':args.get('context'),'action':'spawn'})
 finally:TEMPLATE.reset(token);BUSINESS.reset(business);PRIORITY.reset(priority)

def scheduler_control(action,args):
 agent=CURRENT.get()
 if agent is None:raise ValueError('Authenticated native scheduler context required')
 if not __import__('xiar_hermes.process_execution',fromlist=['configured']).configured(root()):raise ValueError('Durable scheduler not activated')
 if action not in ('list_running','queue_status','cancel'):raise ValueError('Unknown scheduler control')
 body=dict(action=action)
 if action=='cancel':body['run_id']=str(args.get('run_id',''))
 return client(agent,body)
