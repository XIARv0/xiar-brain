# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""External native final-execution gate. Upstream files are never edited; failure is closed, not a hint."""
from pathlib import Path
import contextvars,functools,hashlib,inspect,json,os,stat,threading,weakref
STATE=Path('/etc/xiar/autonomy-state.json')
ACTIVE=contextvars.ContextVar('xiar_autonomy_active',default=())
TICKET=contextvars.ContextVar('xiar_autonomy_ticket',default=None)
_INSTALLED=False
AGENTS=weakref.WeakSet();LOCK=threading.RLock()
def state():
 try:
  info=STATE.lstat()
  if not stat.S_ISREG(info.st_mode) or info.st_uid!=0 or info.st_mode&0o022:raise ValueError('Unsafe emergency state')
  value=json.loads(STATE.read_text())
  if not isinstance(value['stopped'],bool):raise ValueError('Invalid emergency state')
  return value
 except (OSError,ValueError,KeyError):return dict(stopped=True,generation=-1)
def readonly(home=None):
 if state()['stopped']:return True
 if home is None:return False
 try:
  from types import SimpleNamespace
  return bool(rpc(SimpleNamespace(_xiar_execution_home=home),dict(action='status'))['stopped'])
 except Exception:return True
def rpc(agent,body):
 from .execution import client
 return client(agent,body,path='/autonomy')
def guarded(agent,tool,args,fn):
 if agent is None or not isinstance(args,dict):return json.dumps(dict(error='Authenticated native Autonomy context unavailable; execution refused'))
 digest=hashlib.sha256(json.dumps([tool,args],sort_keys=True,allow_nan=False).encode()).hexdigest()
 active=dict(ACTIVE.get())
 if digest in active:
  if readonly(getattr(agent,'_xiar_execution_home',None)) and active[digest]:return json.dumps(dict(error='Emergency stop changed during tool execution; nested effect refused'))
  return fn(args)
 try:
  decision=rpc(agent,dict(action='begin',tool=tool,args=args,session=str(getattr(agent,'session_id','') or '')))
 except Exception:return json.dumps(dict(error='Rust Autonomy authority unavailable; execution refused'))
 if not decision.get('allowed'):return json.dumps(dict(error=decision.get('reason','Rust Autonomy refused execution'),approval_required=decision.get('approval_required',False),request_id=decision.get('request_id')))
 token=ACTIVE.set(tuple({**active,digest:bool(decision.get('mutating',True))}.items()));ticket_token=TICKET.set(decision['request_id']);success=False
 try:
  # Execute-time barrier: a stop that landed between the approval above and this line must still win. Read-only tools are not blocked (a stopped Xiar stays readable);
  # an unreadable or unsafe state counts as stopped.
  if decision.get('mutating',True) and readonly(getattr(agent,'_xiar_execution_home',None)):
   return json.dumps(dict(error='Emergency stop changed before execution; the action was not run'))
  result=fn(args);success=True
  if isinstance(result,str):
   try:
    value=json.loads(result)
    if isinstance(value,dict) and value.get('error'):success=False
   except (ValueError,TypeError):pass
  return result
 finally:
  ACTIVE.reset(token);TICKET.reset(ticket_token)
  try:rpc(agent,dict(action='finish',request_id=decision['request_id'],success=success))
  except Exception:pass # never repeat a completed effect because its audit ACK was lost

def needs_interrupt(current,generation,previous,was_stopped):
 """True when running conversations must be interrupted: a stop is in force and new, or the generation moved without a stop ever being seen (stop+resume inside one poll)."""
 if generation==previous:return False
 return bool(current['stopped']) or not was_stopped

def contracts():
 import model_tools,agent.inline_tool_executors as inline,tools.connectors as connectors
 if not {'name','args'}.issubset(inspect.signature(model_tools.registry.dispatch).parameters):raise RuntimeError('Native final registry dispatch ABI changed')
 if list(inspect.signature(connectors.dispatch_connector_call).parameters)!=['name','arguments','tool_call_id']:raise RuntimeError('Native connector final dispatch ABI changed')
 if list(inspect.signature(inline.resolve_invoke_tool_executor).parameters)!=['agent','function_name']:raise RuntimeError('Native inline executor ABI changed')
 return dict(final_registry=True,connector_dispatch=True,inline=True,acceptance_run=False)
def install():
 global _INSTALLED
 if _INSTALLED:return
 contracts()
 import model_tools,agent.inline_tool_executors as inline,tools.connectors as connectors
 from run_agent import AIAgent
 from .execution import CURRENT
 # Guard handlers AFTER native request/pre-dispatch/execution middleware transforms.
 # Do not approve aliases/bridge envelopes or pre-normalized arguments as separate actions.
 registry_instance=model_tools.registry;original_dispatch=registry_instance.dispatch
 @functools.wraps(original_dispatch)
 def dispatch(name,args,**context):
  return guarded(CURRENT.get(),name,args,lambda final:original_dispatch(name,final,**context))
 registry_instance.dispatch=dispatch
 original_connector=connectors.dispatch_connector_call
 @functools.wraps(original_connector)
 def connector(name,arguments,tool_call_id):
  return guarded(CURRENT.get(),name,arguments,lambda final:original_connector(name,final,tool_call_id))
 connectors.dispatch_connector_call=connector
 original_resolver=inline.resolve_invoke_tool_executor
 @functools.wraps(original_resolver)
 def resolve(agent,function_name):
  executor=original_resolver(agent,function_name)
  if executor is None:return None
  @functools.wraps(executor)
  def bound(actual,args,context):return guarded(actual,function_name,args,lambda final:executor(actual,final,context))
  return bound
 inline.resolve_invoke_tool_executor=resolve
 # Existing imports keep their aliases; replace only exact original function/method identities.
 import sys
 replacements=((original_dispatch,dispatch),(original_connector,connector),(original_resolver,resolve))
 for name,module in tuple(sys.modules.items()):
  if module is None or not (name.startswith(('agent.','tools.')) or name in ('run_agent','model_tools')):continue
  for attr,value in tuple(vars(module).items()):
   for old,new in replacements:
    same_method=inspect.ismethod(value) and inspect.ismethod(old) and value.__self__ is old.__self__ and value.__func__ is old.__func__
    if value is old or same_method:setattr(module,attr,new)
 original_init=AIAgent.__init__
 @functools.wraps(original_init)
 def initialize(self,*a,**kw):
  original_init(self,*a,**kw)
  with LOCK:AGENTS.add(self)
 AIAgent.__init__=initialize
 def emergency_watch():
  previous=state().get('generation',-1);was_stopped=bool(state()['stopped'])
  while True:
   # Poll cadence is configured in the same installed policy, not a tool-controlled delay.
   try:
    import tomllib
    delay=float(tomllib.loads(Path('/etc/xiar/autonomy.toml').read_text())['control']['poll_s'])
    if not 0<delay<=60:raise ValueError('Unsafe emergency poll')
   except (OSError,ValueError,KeyError):delay=1.0
   threading.Event().wait(delay);current=state()
   with LOCK:available=list(AGENTS)
   if not current['stopped'] and available:
    confirmed=None
    for agent in available:
     try:confirmed=rpc(agent,dict(action='status'));break
     except Exception:continue # an expired worker must not mask an available standing-Xiar authority
    current=confirmed if confirmed is not None else dict(stopped=True,generation=-1)
   generation=current.get('generation',-1)
   # The generation changes on every stop and every resume. A stop followed by a resume inside one poll leaves stopped=False, but the generation moved without
   # this loop ever having seen a stop: that is a missed stop, and the running conversations are interrupted just the same.
   if needs_interrupt(current,generation,previous,was_stopped):
    with LOCK:agents=list(AGENTS)
    for agent in agents:
     try:agent.hard_interrupt('XIAR emergency stop; new conversation is read-only until owner resumes',tool_reason='xiar_emergency_stop')
     except Exception:pass
   previous=generation;was_stopped=bool(current['stopped'])
 threading.Thread(target=emergency_watch,daemon=True,name='xiar-autonomy-stop').start();_INSTALLED=True

def emergency_stop():
 from .execution import CURRENT
 agent=CURRENT.get()
 if agent is None:raise ValueError('Standing Xiar execution context required')
 return rpc(agent,dict(action='stop'))