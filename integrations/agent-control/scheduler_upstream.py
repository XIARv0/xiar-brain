# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Durable metadata-only inference ownership. Unknown upstream completion never frees a permit."""
from pathlib import Path
import hashlib,json,os,re,time,tomllib,urllib.parse,urllib.request
import runtime
from managed_auth import bearer
ID=re.compile(r'^[a-f0-9]{32}$')
class NoRedirect(urllib.request.HTTPRedirectHandler):
 def redirect_request(self,*a,**kw):return None
OPENER=urllib.request.build_opener(urllib.request.ProxyHandler({}),NoRedirect())
def process_identity(pid):
 try:return Path('/proc/'+str(int(pid))+'/stat').read_text().rsplit(')',1)[1].split()[19]
 except (OSError,ValueError,IndexError):return ''
class Upstream:
 def __init__(self,scheduler):
  self.s=scheduler;self.store=scheduler.store;self.reply_cutoff=0.0
  with self.store.db() as db:db.executescript("CREATE TABLE IF NOT EXISTS model_requests(id TEXT PRIMARY KEY,job TEXT NOT NULL,connection TEXT NOT NULL,device TEXT NOT NULL,gpu INTEGER NOT NULL,local INTEGER NOT NULL,backend TEXT NOT NULL,state TEXT NOT NULL,pid INTEGER NOT NULL,birth TEXT NOT NULL,created REAL NOT NULL,updated REAL NOT NULL); CREATE INDEX IF NOT EXISTS requests_live ON model_requests(state,job);")
 def connection(self,key):
  rt=tomllib.loads((runtime.ROOT/'config/llm_runtime.toml').read_text());chat=dict(rt['chat']);chat['id']=chat.get('served_name','chat')
  rows={chat['id']:chat,**{k:dict(v,id=k) for k,v in rt.get('connections',{}).items()}}
  if key not in rows:raise ValueError('Inference connection no longer configured')
  c=rows[key];url=str(c.get('base_url') or 'http://'+str(c.get('host',''))+':'+str(c.get('port',''))).rstrip('/');p=urllib.parse.urlsplit(url)
  if p.username or p.password or p.fragment or p.query:raise ValueError('Unsafe configured upstream')
  local=p.hostname in ('localhost','127.0.0.1','::1');fingerprint=hashlib.sha256(json.dumps([key,url,c.get('served_name'),c.get('runner'),c.get('managed_auth_token_file')],sort_keys=True).encode()).hexdigest()
  return c,url,local,fingerprint
 def idle(self,key,fingerprint):
  try:
   c,url,local,current=self.connection(key)
   if not local or current!=fingerprint or c.get('scheduler_idle_probe')!='llama_metrics':return False
   value=dict(c)
   if value.get('managed_auth_token_file'):value['managed_auth_token_file']=value['managed_auth_token_file'].replace('~',runtime.OWNER.pw_dir,1)
   headers=bearer(value,owner_uid=runtime.OWNER.pw_uid);endpoint=url.removesuffix('/v1')+'/metrics'
   with OPENER.open(urllib.request.Request(endpoint,headers=headers),timeout=3) as q:raw=q.read(262145)
   if len(raw)>262144:return False
   metrics={}
   for line in raw.decode().splitlines():
    if not line or line.startswith('#'):continue
    parts=line.split()
    if len(parts)==2 and parts[0] in ('llamacpp:requests_processing','llamacpp:requests_deferred'):metrics[parts[0]]=float(parts[1])
   return len(metrics)==2 and all(v==0 for v in metrics.values())
  except (OSError,ValueError,KeyError):return False
 def rows(self,job=None):
  with self.store.db() as db:return [dict(v) for v in db.execute("SELECT * FROM model_requests WHERE state IN ('open','uncertain')"+(' AND job=?' if job else ''),[job] if job else [])]
 def reconcile(self):
  for row in self.rows():
   if row['state']=='open' and process_identity(row['pid'])!=row['birth']:
    with self.store.db() as db:db.execute("UPDATE model_requests SET state='uncertain',updated=? WHERE id=? AND state='open'",(time.time(),row['id']))
    row['state']='uncertain'
   # A live sender can still open a socket after a transient idle sample. Only a
   # closed/uncertain sender or a proven-dead router may be cleared by idle proof.
   if row['state']=='uncertain' and self.idle(row['connection'],row['backend']):
    with self.store.db() as db:db.execute("UPDATE model_requests SET state='drained',updated=? WHERE id=? AND state='uncertain'",(time.time(),row['id']))
 def cancel_replies(self):
  """Owner's 'stop all model replies now': every reply that began before this moment is told to stop (the router closes its upstream socket; the receipt is
  cleared by the idle proof, never assumed free). Replies that begin afterwards are unaffected, and nothing else (Agents, tools, memory) is touched."""
  with self.s.lifecycle:
   self.reply_cutoff=time.time();return sum(1 for row in self.rows() if row['state']=='open')
 def quiet(self,job):
  self.reconcile();return not self.rows(job)
 def call(self,body,pid):
  action=body.get('action');request=str(body.get('request_id',''))
  if not ID.fullmatch(request):raise ValueError('Stable inference request id required')
  emergency=self.s.emergency()
  if action=='begin' and emergency['stopped'] and (body.get('run_id') or body.get('profile')!='owner'):return dict(allowed=False,cancel=True)
  if action=='check':
   with self.store.db() as db:early=db.execute('SELECT * FROM model_requests WHERE id=?',(request,)).fetchone()
   if early and early['pid']==pid and early['birth']==process_identity(pid) and early['created']<=max(emergency.get('cutoff',0),self.reply_cutoff):return dict(cancel=True)
  with self.s.lifecycle:
   with self.store.db() as db:old=db.execute('SELECT * FROM model_requests WHERE id=?',(request,)).fetchone()
   if action=='begin':
    job=str(body.get('run_id',''));key=str(body.get('connection',''));c,url,local,fingerprint=self.connection(key)
    if old:
     if old['job']!=job or old['connection']!=key or old['pid']!=pid or old['birth']!=process_identity(pid):raise ValueError('Inference id identity changed')
     return dict(allowed=old['state']=='open',closed=old['state']=='drained')
    if job:
     row=self.store.row(job)
     if not row or row['state'] not in ('running','launching','yielding','waiting_children','resuming') or not self.s.live(row):return dict(allowed=False,cancel=True)
     if row['state'] not in ('running','launching'):return dict(allowed=False,wait=True)
     placement=self.store.open(job,'placement',row['placement'])
     if placement['id']!=key:raise ValueError('Inference connection differs from durable lease')
     device=placement['device'];gpu=int(placement.get('gpu',0))
    else:device=str(c.get('device','auto'));gpu=int(c.get('gpu',0))
    self.reconcile();active=self.rows()
    conflict=[v for v in active if v['connection']==key or local and v['local'] and (device=='auto' or v['device']=='auto' or v['device']==device and v['gpu']==gpu)]
    limit=1 if local else int(c.get('dispatch_concurrency',self.s.cfg()['max_light']))
    if len(conflict)>=limit:return dict(allowed=False,wait=True)
    birth=process_identity(pid)
    if not birth:raise ValueError('Router process identity unavailable')
    with self.store.db() as db:db.execute('INSERT INTO model_requests VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',(request,job,key,device,gpu,int(local),fingerprint,'open',pid,birth,time.time(),time.time()))
    return dict(allowed=True)
   if not old or old['pid']!=pid or old['birth']!=process_identity(pid):raise ValueError('Foreign inference receipt')
   if action=='check':
    row=self.store.row(old['job']) if old['job'] else None
    cancel=bool(old['job'] and (not row or row['state'] in ('cancelling','releasing','completed','failed','cancelled','killed_budget') or not self.s.live(row)))
    return dict(cancel=cancel)
   if action=='finish':
    state='drained' if body.get('complete') is True else 'uncertain'
    with self.store.db() as db:db.execute("UPDATE model_requests SET state=?,updated=? WHERE id=? AND state!='drained'",(state,time.time(),request))
    return dict(recorded=True)
   raise ValueError('Unknown inference lifecycle action')
