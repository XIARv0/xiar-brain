# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Native lifecycle remains original; only actual conversation and scoped controls cross the durable boundary."""
import inspect,json,threading,uuid,tomllib
from pathlib import Path

def configured(root):
 path=Path(root)/'config/scheduler.toml'
 return path.is_file() and bool(tomllib.loads(path.read_text())['scheduler']['enabled'])

def run(original,batch,index,task,child):
 from .execution import client,root
 from agent.conversation_loop import run_conversation as native_conversation
 parent=batch.parent_agent;native=child.run_conversation;signature=inspect.signature(native_conversation)
 cfg=tomllib.loads((root()/'config/scheduler.toml').read_text())['scheduler']
 native_steer=child.steer;native_drain=child._drain_pending_steer;outbox=[];controls_lock=threading.RLock()
 def steer(text):
  if not isinstance(text,str) or not text.strip() or len(text.strip())>cfg['max_steer_chars']:return False
  with controls_lock:
   if len(outbox)>=cfg['max_controls']:return False
   if not native_steer(text):return False
   outbox.append(dict(id=uuid.uuid4().hex,text=text.strip(),accepted=False,delivered=False));return True
 def drain():
  # Registry closure still owns the acceptance race. Preserve exact unconfirmed texts
  # if the native parent times out before the remote worker returns its receipt.
  with controls_lock:
   native_drain();return '\n'.join(v['text'] for v in outbox if not v['delivered']) or None
 def remote(*a,**kw):
  bound=signature.bind(child,*a,**kw);conversation=dict(bound.arguments);conversation.pop('agent');stream=conversation.pop('stream_callback',None)
  from .worker import CONVERSATION_FIELDS
  if set(conversation)-CONVERSATION_FIELDS:raise ValueError('Native conversation ABI no longer matches durable bridge')
  job=uuid.uuid4().hex;child._xiar_durable_run=job
  from tools.terminal_tool import get_session_cwd
  from tools.delegate_tool_config import _get_max_spawn_depth,_get_orchestrator_enabled
  body=dict(action='submit',run_id=job,brief=task['goal'],template_id=getattr(child,'_xiar_requested_template',''),spec=getattr(child,'_xiar_worker_spec',{}),conversation=conversation,business=getattr(child,'_xiar_requested_business',''),priority=getattr(child,'_xiar_requested_priority','normal'),workspace=str(get_session_cwd(conversation.get('task_id'))),native=dict(max_spawn_depth=_get_max_spawn_depth(),orchestrator_enabled=_get_orchestrator_enabled(),context=task.get('context'),subagent_id=getattr(child,'_subagent_id','')))
  json.dumps(body,allow_nan=False);cursor=0;submitted=False;cancelled=False
  while True:
   cancel_reply=None
   if getattr(child,'_interrupt_requested',False) and not cancelled:
    try:cancel_reply=client(parent,dict(action='cancel',run_id=job));cancelled=True
    except (OSError,ValueError):
     with controls_lock:pending='\n'.join(v['text'] for v in outbox if not v['delivered'])
     return dict(completed=False,interrupted=True,failed=True,error='Cancellation not confirmed; durable run id retained and resources NOT released',pending_steer=pending)
   try:
    if cancel_reply is not None:value=cancel_reply;submitted=True
    elif not submitted:value=client(parent,body);submitted=True
    else:value=client(parent,dict(action='status',run_id=job,cursor=cursor))
    child._touch_activity('XIAR durable scheduler: '+str(value.get('state','waiting')))
    if getattr(child,'_interrupt_requested',False) and not cancelled:client(parent,dict(action='cancel',run_id=job));cancelled=True
    with controls_lock:
     for receipt in value.get('controls',[]):
      for local in outbox:
       if local['id']==receipt['id']:local.update(accepted=True,delivered=bool(receipt['delivered']))
     pending=[dict(v) for v in outbox if not v['accepted']] if value.get('state') not in ('completed','failed','cancelled','killed_budget') else []
    for control in pending:
     reply=client(parent,dict(action='steer',run_id=job,control_id=control['id'],text=control['text']))
     if reply.get('accepted'):
      with controls_lock:
       for local in outbox:
        if local['id']==control['id']:local['accepted']=True
    for row in value.get('events',[]):
     cursor=max(cursor,int(row['cursor']));event=row['event']
     if event.get('kind')=='stream' and callable(stream):stream(event.get('value',''))
     elif event.get('kind')=='progress':
      callback=getattr(child,'tool_progress_callback',None);data=event.get('value',{})
      if callable(callback):callback(*data.get('args',[]),**data.get('kwargs',{}))
    if value.get('state') in ('completed','failed','cancelled','killed_budget'):
     output=value.get('output') or {'result':{'completed':False,'failed':True,'error':'Durable worker ended without a result'}}
     numeric=('total_input_tokens','total_output_tokens','total_cache_read_tokens','total_cache_write_tokens','api_calls','_last_activity_ts','session_prompt_tokens','session_completion_tokens','session_estimated_cost_usd')
     for name,number in output.get('metrics',{}).items():
      if name in numeric and isinstance(number,(int,float)):setattr(child,name,number)
     if isinstance(output.get('metrics',{}).get('session_cost_status'),str):child.session_cost_status=output['metrics']['session_cost_status']
     with controls_lock:
      missing=[v['text'] for v in outbox if v['id'] not in output.get('accepted_controls',[])]
      if missing:output['result']['pending_steer']='\n'.join([output['result'].get('pending_steer',''),*missing]).strip()
      # Result now carries every missed/unconfirmed steer; do not append it twice at native closure.
      for local in outbox:local['delivered']=True
      native_drain()
     child._xiar_workspace_handoff=output.get('workspace_handoff');return output['result']
   except (OSError,ValueError):pass # stable submit/control ids survive lost acknowledgements; no early release
   threading.Event().wait(float(cfg['poll_s']))
 child.run_conversation=remote;child.steer=steer;child._drain_pending_steer=drain
 try:
  entry=original(batch,index,task,child)
  if isinstance(entry,dict):
   if getattr(child,'_xiar_workspace_handoff',None):entry['workspace_handoff']=child._xiar_workspace_handoff
   if getattr(child,'_xiar_durable_run',None):entry['durable_run_id']=child._xiar_durable_run
  return entry
 finally:child.run_conversation=native;child.steer=native_steer;child._drain_pending_steer=native_drain
