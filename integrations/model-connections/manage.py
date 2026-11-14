# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Connection lifecycle through the existing Laya launcher or a private provider adapter."""
import hashlib
import json,os,pathlib,re,subprocess,sys,tomllib,urllib.request
ROOT=pathlib.Path(os.environ.get('XIAR_ROOT',pathlib.Path(__file__).resolve().parents[2]))
def native(command,args):
 p=subprocess.run([str(ROOT/'core/rust/target/release/xiar-settings-core')],input=json.dumps(dict(command=command,args=args)),text=True,capture_output=True,check=True,timeout=30)
 result=json.loads(p.stdout)
 if 'error'in result:raise ValueError(result['error'])
 return result['result']
def unit(id):
 if not re.fullmatch('[A-Za-z0-9_-]{1,64}',id):raise ValueError('Invalid connection ID')
 return 'xiar-connection-'+id+'.service'
def active(name):return subprocess.run(['systemctl','--user','is-active','--quiet',name]).returncode==0
def process(action,id):
 name=unit(id)
 if action=='forget':
  target=pathlib.Path.home()/'.config/systemd/user'/name
  if target.exists():
   subprocess.run(['systemctl','--user','disable','--now',name],check=True,timeout=90);target.unlink();subprocess.run(['systemctl','--user','daemon-reload'],check=True,timeout=30)
  return dict(removed=True)
 runtime=tomllib.loads((ROOT/'config/llm_runtime.toml').read_text());c=runtime.get('connections',{}).get(id)
 if not c:raise ValueError('Connection is not configured')
 panel=tomllib.loads((ROOT/'config/settings_panel.toml').read_text());local=c.get('type')=='local'
 if action=='stop':
  if runtime.get('chat',{}).get('source_connection')==id:raise ValueError('Select another main connection before stopping this one')
  subprocess.run(['systemctl','--user','stop',name],check=True,timeout=90);return dict(active=active(name))
 if action=='status':return dict(active=active(name))
 if action=='health':
  port=int(c['port'] if local else c['proxy_port'])
  with urllib.request.urlopen('http://127.0.0.1:'+str(port)+'/v1/models',timeout=10) as response:models=json.load(response)
  model=c.get('served_name') or c.get('model');ready=any(row.get('id')==model for row in models.get('data',[]))
  return dict(active=active(name),ready=ready,model=model)
 if action not in ('start','main'):raise ValueError('Unknown lifecycle action')
 port=int(c.get('port',0) if local else c.get('proxy_port',0))
 if not 1023<port<65536:raise ValueError('Choose a serving port or provider proxy port from 1024 to 65535')
 for key,default_unit in [('chat','xiar-chat'),('embedding','xiar-embed')]:
  original=runtime.get(key,{})
  if original.get('port')==port and not original.get('source_connection') and active(panel.get('runtime',{}).get(key+'_unit',default_unit)):raise ValueError('Port belongs to an active runtime; choose a different serving port')
 for other,d in runtime.get('connections',{}).items():
  if other==id:continue
  other_port=d.get('port') if d.get('type')=='local' else d.get('proxy_port')
  if other_port==port and active(unit(other)):raise ValueError('Another running connection uses this port')
 if local and c.get('mistralrs_form')=='builtin':
  candidates=[runtime.get('chat',{})]+[d for key,d in runtime.get('connections',{}).items() if key!=id and active(unit(key))]
  for d in candidates:
   if d.get('mistralrs_form')=='builtin' and (d.get('model_path'),d.get('device','auto'))!=(c.get('model_path'),c.get('device','auto')):raise ValueError('Built-in engine: one model/device including active chat and memory')
 prepared=native('connection_prepare',dict(id=id));cfg=prepared['config_path']
 if any(ch in str(ROOT)+cfg for ch in '\n\r%'):raise ValueError('Unsafe service path')
 if local:
  if c['kind']=='chat':python='/usr/bin/python3';script=ROOT/'integrations/llm-local/chat/serve_chat.py'
  elif c['kind']=='embedding':python=os.path.expanduser(panel.get('runtime',{}).get('embedding_python','~/.local/share/xiar/embed-venv/bin/python'));script=ROOT/'integrations/llm-local/embed-server/xiar_embed_server.py'
  else:raise ValueError('No installed serving backend for this model kind')
  command='"'+python+'" "'+str(script)+'"'
 else:command='/usr/bin/python3 "'+str(ROOT/'integrations/model-connections/proxy.py')+'" '+id
 data='[Unit]\nDescription=XIAR configured model connection\n[Service]\nType=exec\nWorkingDirectory='+str(ROOT)+'\nEnvironment="XIAR_ROOT='+str(ROOT)+'"\nEnvironment="XIAR_LLM_RUNTIME_CONFIG='+cfg+'"\nExecStart='+command+'\nRestart=on-failure\nRestartSec=5\n[Install]\nWantedBy=default.target\n'
 target=pathlib.Path.home()/'.config/systemd/user'/name;target.parent.mkdir(parents=True,exist_ok=True);target.write_text(data)
 subprocess.run(['systemctl','--user','daemon-reload'],check=True,timeout=30)
 stamp=ROOT/'data/connection-runtime'/id;stamp.parent.mkdir(parents=True,exist_ok=True)
 digest=hashlib.sha256(json.dumps(c,sort_keys=True).encode()).hexdigest()
 if action=='start' or not active(name):
  subprocess.run(['systemctl','--user','enable',name],check=True,timeout=30)
  subprocess.run(['systemctl','--user','restart',name],check=True,timeout=90);stamp.write_text(digest)
 if action=='main' and (not stamp.exists() or stamp.read_text()!=digest):raise ValueError('Connection configuration changed; Start/restart it before choosing main')
 if action=='main':
  if c.get('kind')!='chat':raise ValueError('Main connection must be a chat/instruct model')
  if int(c.get('context_length',0))<64000:raise ValueError('Hermes main connections require at least 64000 context tokens; memory jobs can use smaller models')
  # A connection must actually serve the selected model before switching Hermes.
  health=process('health',id)
  if not health['ready']:raise ValueError('Connection is starting or not serving its selected model; main connection unchanged')
  selected={k:v for k,v in c.items() if k not in ('id','proxy_port','credential_env','credential_ref','max_response_bytes','max_request_bytes','timeout_s','revision')}
  selected.update(type='openai',base_url='http://127.0.0.1:'+str(port),host='127.0.0.1',port=port,source_connection=id)
  native('runtime_set',dict(chat=selected))
  subprocess.run(['bash',str(ROOT/'integrations/hermes-xiar/set-local-chat-provider.sh')],check=True,capture_output=True,timeout=90)
  subprocess.run([sys.executable,str(ROOT/'integrations/hermes-xiar/set-thinking.py')],check=True,capture_output=True,timeout=90)
  if runtime.get('chat',{}).get('source_connection') is None:subprocess.run(['systemctl','--user','stop',panel.get('runtime',{}).get('chat_unit','xiar-chat')],check=True,timeout=90)
 return dict(active=active(name),main=action=='main',message='Startup requested; readiness is a separate real provider check')
if __name__=='__main__':
 try:print(json.dumps(process(sys.argv[1],sys.argv[2])))
 except Exception as e:
  print(json.dumps({'error':str(e) if isinstance(e,ValueError) else 'Connection lifecycle failed; no provider response or credentials logged'}));sys.exit(2)
