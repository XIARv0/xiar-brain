# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Current external-only native config overlay; no stock source/home/venv writes."""
from pathlib import Path
import copy,functools,json,os,shlex,sys,tomllib
_INSTALLED=False
PROVIDER='xiar-routed'
def settings(root):
 runtime=tomllib.loads((root/'config/llm_runtime.toml').read_text());router=tomllib.loads((root/'config/model_router.toml').read_text())['model_router'];adapter=tomllib.loads((root/'config/hermes_adapter.toml').read_text())
 enabled=bool(router['enabled'] and adapter.get('laya',{}).get('enabled',True) and adapter.get('laya',{}).get('jobs',{}).get('model_routing',False))
 migration=tomllib.loads((root/'config/scheduler.toml').read_text())['scheduler']
 enabled=bool(enabled or migration['enabled'] or not migration.get('legacy_native_fallback',True))
 if router['host']!='127.0.0.1':raise ValueError('Private model transport must remain loopback')
 return runtime,router,enabled

def binding(root,home=None,marker=None):
 runtime,router,enabled=settings(root)
 if not enabled:return None
 if home is None:
  from hermes_constants import get_hermes_home
  home=Path(get_hermes_home())
 home=Path(home);marker_path=home/'.xiar-identity.json'
 if marker is None and marker_path.is_file():marker=json.loads(marker_path.read_text())
 if marker:
  profile=marker['profile'];agent=marker['agent'];credential=home/'model-router-token';suffix='/p/'+profile
 else:
  adapter=tomllib.loads((root/'config/hermes_adapter.toml').read_text())
  profile='owner';agent=adapter['identity']['agent_id'];credential=Path(os.path.expanduser(router['token_file']));suffix=''
 job=os.environ.get('XIAR_EXECUTION_JOB','')
 chat=runtime['chat']
 if job:
  import re
  if not re.fullmatch(r'[a-f0-9]{32}',job):raise ValueError('Invalid immutable worker binding')
  payload=json.loads(Path(os.environ['XIAR_TASK_INPUT']).read_text())
  if payload['run_id']!=job:raise ValueError('Worker task identity mismatch')
  credential=Path(os.environ['CREDENTIALS_DIRECTORY'])/'model-router-token';suffix='/j/'+job
  match=next((v for key,v in runtime.get('connections',{}).items() if key==payload['placement']['id']),None)
  if match:chat={**chat,**match}
  chat={**chat,'served_name':payload['placement']['model']}
 thinking='auto'
 for value in [runtime.get('agents',{}).get(agent,{}).get('thinking','inherit'),runtime.get('agents',{}).get('default',{}).get('thinking','inherit'),runtime.get('llms',{}).get(chat['served_name'],{}).get('thinking',chat.get('thinking','auto'))]:
  if value!='inherit':thinking=value;break
 if thinking not in ('auto','on','off'):raise ValueError('Invalid configured thinking mode')
 provider={'api':f"http://{router['host']}:{int(router['port'])}{suffix}/v1",'default_model':chat['served_name'],'key_cmd':shlex.join(['/usr/bin/python3',str(Path(__file__).resolve().with_name('model_credentials.py')),'--file',str(credential)])}
 if thinking!='auto':provider['extra_body']={'chat_template_kwargs':{'enable_thinking':thinking=='on'},'enable_thinking':thinking=='on'}
 return dict(provider=provider,profile=profile,model=chat['served_name'],context=chat['context_length'])

def apply(config,root,home=None,marker=None):
 value=binding(root,home,marker)
 if value is None:return config
 out=copy.deepcopy(config);out.setdefault('model',{}).update(provider=PROVIDER,base_url=value['provider']['api'],default=value['model'],context_length=value['context'])
 # Any unrelated pinned key must not override our per-profile credential command.
 for key in ('api_key','key_env','api_key_env'):out['model'].pop(key,None)
 from .execution import LEASE
 lease=LEASE.get()
 if lease:value['provider']['extra_headers']={'X-Xiar-Dispatch-Run':lease}
 out.setdefault('providers',{})[PROVIDER]=value['provider']
 return out

def install(root):
 global _INSTALLED
 if _INSTALLED:return
 import hermes_cli.config as cfg
 import hermes_cli.runtime_provider as rp
 originals={name:getattr(cfg,name) for name in ('load_config','load_config_readonly')}
 wrappers={}
 for name,original in originals.items():
  def make(fn):
   @functools.wraps(fn)
   def loaded(*a,**kw):return apply(fn(*a,**kw),root)
   return loaded
  wrappers[name]=make(original);setattr(cfg,name,wrappers[name])
 # Runtime provider has its own loader; route both it and already-imported aliases.
 original_rp=rp.load_config
 @functools.wraps(original_rp)
 def runtime_config(*a,**kw):return apply(original_rp(*a,**kw),root)
 rp.load_config=runtime_config
 for module in tuple(sys.modules.values()):
  if module is None:continue
  namespace=getattr(module,'__dict__',{})
  for name,original in originals.items():
   if namespace.get(name) is original:namespace[name]=wrappers[name]
 # Session /model overrides re-resolve by saved provider name 'custom' + base_url, which drops key_cmd and sends
 # the 'no-key-required' placeholder (turn 2+ got HTTP 401). Re-attach the private credential command for our router URL.
 original_resolve=rp.resolve_runtime_provider
 sources={}
 @functools.wraps(original_resolve)
 def resolve_routed(*a,**kw):
  result=original_resolve(*a,**kw)
  try:
   value=binding(root)
   if value and isinstance(result,dict) and str(result.get('base_url','')).rstrip('/')==value['provider']['api'].rstrip('/') and not callable(result.get('api_key')):
    command=value['provider']['key_cmd']
    if command not in sources:
     from agent.command_token_source import build_command_token_provider
     sources[command]=build_command_token_provider(command,PROVIDER)
    if sources[command] is not None:result=dict(result,api_key=sources[command])
  except Exception:pass
  return result
 rp.resolve_runtime_provider=resolve_routed
 for module in tuple(sys.modules.values()):
  if module is None:continue
  namespace=getattr(module,'__dict__',{})
  if namespace.get('resolve_runtime_provider') is original_resolve:namespace['resolve_runtime_provider']=resolve_routed
 _INSTALLED=True
