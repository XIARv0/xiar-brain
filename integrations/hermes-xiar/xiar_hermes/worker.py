# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""One real native SDK conversation per kernel-bounded process. No pickle, shell exec or credential serialization."""
from pathlib import Path
import inspect,json,os,sys,threading,time
SPEC_FIELDS={'max_iterations','max_tokens','enabled_toolsets','disabled_toolsets','ephemeral_system_prompt','prefill_messages','reasoning_config','session_id','parent_session_id'}
CONVERSATION_FIELDS={'user_message','system_message','conversation_history','task_id','persist_user_message','persist_user_timestamp','persist_user_display_kind','persist_user_display_metadata','persist_user_platform_id','turn_author'}
def contracts():
 from run_agent import AIAgent
 from agent.conversation_loop import run_conversation
 init=set(inspect.signature(AIAgent.__init__).parameters);conversation=set(inspect.signature(run_conversation).parameters)
 forced={'model','quiet_mode','side_agent','platform','skip_context_files','skip_memory','skip_background_review','cwd','user_id','tool_progress_callback'}
 if not forced.issubset(init):raise RuntimeError('Native forced worker constructor ABI changed')
 if not SPEC_FIELDS.issubset(init) or not CONVERSATION_FIELDS.issubset(conversation):raise RuntimeError('Native process worker contract changed')
 if not all(hasattr(AIAgent,k) for k in ('close','steer','_drain_pending_steer','_touch_activity')):raise RuntimeError('Native worker cleanup contract changed')
 from tools.delegate_tool_progress import _build_child_system_prompt
 if not {'goal','context','workspace_path','role','max_spawn_depth','child_depth'}.issubset(inspect.signature(_build_child_system_prompt).parameters):raise RuntimeError('Native scoped prompt ABI changed')
 return {'constructor_fields':len(SPEC_FIELDS),'conversation_fields':len(CONVERSATION_FIELDS),'worker_contract':True,'acceptance_run':False}
def atomic_result(value):
 root=Path(os.environ['XIAR_TASK_OUTPUT']);p=root/'result.json';tmp=root/'result.new'
 fd=os.open(tmp,os.O_CREAT|os.O_EXCL|os.O_WRONLY|os.O_NOFOLLOW,0o600)
 with os.fdopen(fd,'w') as out:json.dump(value,out,allow_nan=False);out.flush();os.fsync(out.fileno())
 os.replace(tmp,p)
def main():
 source=Path(os.environ['XIAR_NATIVE_SOURCE']);sys.path.insert(0,str(source))
 root=Path(os.environ['XIAR_ROOT']);path=Path(os.environ['XIAR_TASK_INPUT']);cfg=json.loads(path.read_text())
 if cfg['run_id']!=os.environ['XIAR_EXECUTION_JOB'] or set(cfg['spec'])-SPEC_FIELDS or set(cfg['conversation'])-CONVERSATION_FIELDS:raise ValueError('Worker contract refused')
 from .model_overlay import install as install_model
 from .native_profiles import install as install_profiles
 from .execution import install as install_execution,client
 from .autonomy import install as install_autonomy
 install_model(root);install_execution();install_autonomy();install_profiles()
 from run_agent import AIAgent
 from hermes_cli.runtime_provider import resolve_runtime_provider
 resolved=resolve_runtime_provider(requested='xiar-routed',target_model=cfg['placement']['model']);accepted=set(inspect.signature(AIAgent.__init__).parameters)
 kwargs={k:v for k,v in resolved.items() if k in accepted};kwargs.update(cfg['spec'])
 kwargs.update(model=cfg['placement']['model'],quiet_mode=True,side_agent=True,platform='subagent',skip_context_files=True,skip_memory=True,skip_background_review=True,cwd='/var/lib/xiar-agent/workspace',user_id=cfg['principal']['user'])
 from tools.delegate_tool_progress import _build_child_system_prompt
 native_prompt=_build_child_system_prompt(cfg['brief'],cfg['native'].get('context'),workspace_path='/var/lib/xiar-agent/workspace',role=cfg['native']['role'],max_spawn_depth=cfg['native']['max_spawn_depth'],child_depth=cfg['native']['depth'])
 context=cfg['template']['context'].replace('{business}',cfg['principal'].get('business') or '{business}')
 kwargs['ephemeral_system_prompt']=native_prompt+'\n'+cfg.get('core','')+'\n[Configured XIAR template]\n'+context
 lock=threading.Lock();agent=None
 def event(kind,value):
  if agent is None:return
  try:
   with lock:client(agent,{'action':'event','run_id':cfg['run_id'],'event':dict(kind=kind,value=value)})
  except Exception:pass # no prompt/credential logging; result file survives a broker outage
 def progress(*a,**kw):
  try:event('progress',{'args':list(a),'kwargs':kw})
  except Exception:pass
 kwargs['tool_progress_callback']=progress
 control_stop=threading.Event();control_thread=None;control_lock=threading.RLock();pending_controls=[];delivered=set();control_cursor=0;native_drain=None
 try:
  agent=AIAgent(**kwargs);agent._delegate_depth=int(cfg.get('native',{}).get('depth',1));agent._delegate_role=str(cfg.get('native',{}).get('role','leaf'));agent._subagent_id=cfg.get('native',{}).get('subagent_id')
  # Same noninteractive approval initializer the native delegate worker uses. Product tier enforcement is Phase8.2.
  from tools.delegate_tool import _set_subagent_approval_cb,_get_subagent_approval_callback
  _set_subagent_approval_cb(_get_subagent_approval_callback())
  native_drain=agent._drain_pending_steer
  def drain():
   with control_lock:
    text=native_drain()
    if text:delivered.update(v['id'] for v in pending_controls);pending_controls.clear()
    return text
  agent._drain_pending_steer=drain
  def pull():
   nonlocal control_cursor
   while not control_stop.is_set():
    try:
     with control_lock:receipts=list(delivered)
     value=client(agent,dict(action='control_pull',run_id=cfg['run_id'],cursor=control_cursor,delivered=receipts))
     for row in value.get('controls',[]):
      with control_lock:
       if agent.steer(row['text']):pending_controls.append(row)
       control_cursor=max(control_cursor,int(row['seq']))
    except Exception:pass
    control_stop.wait(1.0)
  control_thread=threading.Thread(target=pull,daemon=True,name='xiar-worker-steering');control_thread.start()
  conversation=dict(cfg['conversation']);conversation['stream_callback']=lambda text:event('stream',str(text))
  result=agent.run_conversation(**conversation)
  control_stop.set();control_thread.join(timeout=35)
  with control_lock:receipts=list(delivered)
  # Root closes/reconciles the durable control journal after kernel quiescence.
  # A lost transport acknowledgement must not discard a valid native result.
  control_reconciliation='deferred_to_root'
  if not control_thread.is_alive():
   try:
    client(agent,dict(action='control_close',run_id=cfg['run_id'],cursor=0,delivered=receipts));control_reconciliation='closed'
   except Exception:pass
  names=('total_input_tokens','total_output_tokens','total_cache_read_tokens','total_cache_write_tokens','api_calls','_last_activity_ts','session_prompt_tokens','session_completion_tokens','session_estimated_cost_usd')
  metrics={k:getattr(agent,k) for k in names if isinstance(getattr(agent,k,None),(int,float))}
  if isinstance(getattr(agent,'session_cost_status',None),str):metrics['session_cost_status']=agent.session_cost_status
  atomic_result({'result':result,'metrics':metrics,'delivered_controls':receipts,'control_reconciliation':control_reconciliation})
 except Exception as e:atomic_result({'result':{'completed':False,'failed':True,'error':'Native bounded worker failed: '+type(e).__name__},'metrics':{}})
 finally:
  control_stop.set()
  if control_thread is not None:control_thread.join(timeout=35)
  if agent is not None:
   try:agent.close()
   except Exception:pass
if __name__=='__main__':main()
