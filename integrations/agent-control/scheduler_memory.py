# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Root-only memory adapter. Workers receive only their job-purpose proxy capability."""
from pathlib import Path
import paths as xiar_paths
import hashlib,hmac,json,os,re,secrets,stat,tomllib,urllib.error,urllib.request
import runtime
VERIFIER=xiar_paths.scheduler_memory_sha()

def service_key(base):
 p=base/'memory-service.key'
 try:
  fd=os.open(p,os.O_CREAT|os.O_EXCL|os.O_WRONLY|os.O_NOFOLLOW,0o600)
  with os.fdopen(fd,'w') as out:out.write(secrets.token_hex(32));out.flush();os.fsync(out.fileno())
 except FileExistsError:pass
 st=p.stat()
 if p.is_symlink() or st.st_uid!=0 or st.st_mode&0o077:raise ValueError('Unsafe scheduler service capability')
 key=p.read_text().strip()
 if len(key)!=64:raise ValueError('Invalid scheduler service capability')
 digest=hashlib.sha256(key.encode()).hexdigest()+'\n'
 if not VERIFIER.exists() or VERIFIER.read_text()!=digest:runtime.atomic_root(VERIFIER,digest.encode(),0o644)
 return key

def request(path,body,headers):
 cfg=tomllib.loads((runtime.ROOT/'config/memory_api.toml').read_text());bind=cfg['server']['bind']
 if not re.fullmatch(r'127\.0\.0\.1:[0-9]{1,5}',bind) or not 0<int(bind.rsplit(':',1)[1])<65536:raise ValueError('Memory backend must remain loopback')
 raw=json.dumps(body,allow_nan=False).encode()
 if len(raw)>int(cfg['server']['max_body_bytes']):raise ValueError('Memory request exceeds backend bound')
 req=urllib.request.Request('http://'+bind+path,data=raw,headers={'Content-Type':'application/json',**headers},method='POST')
 with urllib.request.build_opener(urllib.request.ProxyHandler({}),NoRedirect()).open(req,timeout=30) as q:
  raw=q.read(16777217)
  if len(raw)>16777216:raise ValueError('Memory response exceeds bound')
  return q.status,json.loads(raw)
class NoRedirect(urllib.request.HTTPRedirectHandler):
 def redirect_request(self,*a,**kw):return None

def adapter(who):
 if who['profile']=='owner':return tomllib.loads((runtime.ROOT/'config/hermes_adapter.toml').read_text())
 p=runtime.Profiles(runtime.ROOT);row=p.state()['scopes'].get(who['profile'])
 if not row or not row.get('enabled') or any(row[k]!=who[k] for k in ('agent','user','business')):raise ValueError('Memory scope revoked')
 return json.loads((p.validate(row)/'adapter.json').read_text())

def proxy(who,path,body,supplied):
 allowed={'/v1/episodic/get','/v1/episodic/list','/v1/semantic/search','/v1/semantic/get','/v1/semantic/list','/v1/semantic/insert','/v1/jobs/enqueue','/v1/knowledge/list'}
 if path not in allowed or not who.get('business'):raise ValueError('Worker memory operation/scope refused')
 cfg=adapter(who)
 if not cfg['memory']['enabled']:raise ValueError('Memory disabled for this Agent')
 expected={'x-agent-id':who['agent'],'x-end-user-id':who['user'],'x-business-id':who['business']}
 for key,value in supplied.items():
  if key in ('x-read-agent-id','x-read-user-id'):
   if value!=who['agent' if key=='x-read-agent-id' else 'user']:raise ValueError('Worker cross-Agent memory refused')
  elif key not in expected or value!=expected[key]:raise ValueError('Worker memory identity mismatch')
 p=Path(os.path.expandvars(str(cfg['memory']['token_file']).replace('~',runtime.OWNER.pw_dir,1)))
 if not p.is_absolute():p=runtime.ROOT/p
 st=p.stat()
 if p.is_symlink() or not stat.S_ISREG(st.st_mode) or st.st_mode&0o077:raise ValueError('Private memory capability unavailable')
 key=p.read_text().strip()
 return request(path,body,{'Authorization':'Bearer '+key,**expected})

def catalogue(who,business,limit):
 """Count-only memory catalogue of the standing Xiar for one business. Never returns an entry, a lesson, trainer data or a Vault value."""
 if who.get('profile')!='owner' or who.get('run_id'):raise ValueError('Standing Xiar only')
 cfg=adapter(who)
 if not cfg['memory']['enabled']:return dict(memory_enabled=False,counts_only=True)
 identity=cfg['identity']
 p=Path(os.path.expandvars(str(cfg['memory']['token_file']).replace('~',runtime.OWNER.pw_dir,1)))
 st=p.stat()
 if p.is_symlink() or not stat.S_ISREG(st.st_mode) or st.st_mode&0o077:raise ValueError('Private memory capability unavailable')
 headers={'Authorization':'Bearer '+p.read_text().strip(),'x-agent-id':identity['agent_id'],'x-end-user-id':identity['end_user_id'],'x-business-id':business}
 out=dict(business=business,agent=identity['agent_id'],memory_enabled=True,counts_only=True,kinds=[])
 # a count must never CREATE a store: the memory API opens (and so creates) the scope it is asked about, so only ask about one that already exists
 api=tomllib.loads((runtime.ROOT/'config/memory_api.toml').read_text())
 tenant=runtime.ROOT/str(api['store']['root'])/'agents'/identity['agent_id']/'users'/identity['end_user_id']/'tenants'/business
 if tenant.is_symlink() or not tenant.is_dir():return dict(out,exists=False)
 try:status,semantic=request('/v1/semantic/list',dict(query='',limit=1),headers)
 except urllib.error.HTTPError as e:raise ValueError('Memory catalogue unavailable for this business (HTTP '+str(e.code)+')') from None
 out['kinds'].append(dict(kind='semantic',scope='business',count=int(semantic.get('total',0)),about='Business facts for this business.'))
 status,episodic=request('/v1/episodic/list',dict(limit=limit),headers)
 shown=len(episodic.get('items',[]))
 out['kinds'].append(dict(kind='episodic',scope='business',count=shown,at_least=shown>=limit,about='Recent events for this business (counted up to the bound).'))
 status,documents=request('/v1/knowledge/list',{},headers)
 listed=documents.get('documents',documents.get('items',[])) if isinstance(documents,dict) else documents
 out['kinds'].append(dict(kind='knowledge_documents',scope='business',count=len(listed) if isinstance(listed,list) else 0,about='Documents added to this business.'))
 return out

def lifecycle(base,job,who,state,template,memory_enabled):
 key=service_key(base)
 body=dict(run_id=job,agent=who['agent'],user=who['user'],business=who.get('business',''),state=state,template=template,archive=bool(memory_enabled and who.get('business')))
 return request('/v1/scheduler/lifecycle',body,{'Authorization':'Bearer '+key})[1]

def core(base,who):
 key=service_key(base)
 value=request('/v1/scheduler/core',dict(agent=who['agent'],user=who['user']),{'Authorization':'Bearer '+key})[1]
 blocks=value.get('blocks',[])
 if not isinstance(blocks,list) or len(blocks)>2:raise ValueError('Invalid allowed Core response')
 return '\n\n'.join('[Allowed Core '+v['label']+']\n'+v['value'] for v in blocks if v.get('label') in ('human','persona') and isinstance(v.get('value'),str))
