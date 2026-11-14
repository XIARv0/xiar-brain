# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Root-owned Autonomy coordinator. Canonical decisions in Rust; approval arguments AEAD-sealed."""
from pathlib import Path
import hashlib,json,secrets,stat,subprocess,threading,time,tomllib
import paths as xiar_paths
import runtime
import scheduler_memory as memory
from scheduler_store import ACTIVE,TERMINAL
from autonomy_undo import Undo
import autonomy_effects as effects
import autonomy_permissions as permissions
STATE=None  # resolved by state_file()
def state_file():
 return xiar_paths.state_path()
def status():
 try:
  path=state_file();info=path.lstat()
  if not stat.S_ISREG(info.st_mode) or info.st_uid not in trusted_uids() or info.st_mode&0o022:raise ValueError('Unsafe Autonomy state')
  value=json.loads(path.read_text());assert isinstance(value['stopped'],bool);return value
 except (OSError,ValueError,KeyError,AssertionError):return dict(stopped=True,generation=-1,cutoff=time.time(),unavailable=True)
def trusted_uids():
 return xiar_paths.trusted_uids()
class Autonomy:
 def __init__(self,scheduler):
  self.s=scheduler;self.s.safety=self;self.stop_latch=False;self.stop_cutoff=0;self.store=scheduler.store;self.lock=threading.RLock();self.emergency_lock=threading.RLock();self.undo=Undo(self.cfg)
  with self.store.db() as db:
   db.executescript('CREATE TABLE IF NOT EXISTS autonomy_requests(id TEXT PRIMARY KEY,scope TEXT NOT NULL,job TEXT NOT NULL,session TEXT NOT NULL,digest TEXT NOT NULL,tool TEXT NOT NULL,status TEXT NOT NULL,created REAL NOT NULL,expires REAL NOT NULL,generation INTEGER NOT NULL,body BLOB NOT NULL,snapshot INTEGER); CREATE INDEX IF NOT EXISTS autonomy_scope ON autonomy_requests(scope,status); CREATE TABLE IF NOT EXISTS autonomy_audit(id TEXT PRIMARY KEY,body BLOB NOT NULL,audited INTEGER NOT NULL DEFAULT 0);')
   row=db.execute("SELECT value FROM state WHERE key='autonomy.emergency'").fetchone();value=json.loads(row[0]) if row else dict(stopped=False,generation=0,cutoff=0)
   if not row:db.execute("INSERT INTO state VALUES('autonomy.emergency',?)",(json.dumps(value),))
  self.publish(value)
  self.recover_permissions()
  try:self.reconcile_orphans()
  except Exception:pass
  from autonomy_drive import Drive
  self.drive=Drive(self)
  def retry_audit():
   while True:
    try:self.flush()
    except Exception:pass
    try:self.reconcile_orphans()
    except Exception:pass
    threading.Event().wait(float(self.cfg()['control']['poll_s']))
  threading.Thread(target=retry_audit,daemon=True,name='xiar-autonomy-audit-outbox').start()
 def recover_permissions(self):
  # Root policy replacement is atomic. Resolve a crash from exact sealed before/after policy,
  # never replay a permission widening or infer success from an unrelated policy edit.
  actual=self.cfg()
  with self.lock,self.store.db() as db:
   rows=list(db.execute("SELECT * FROM autonomy_requests WHERE session='native-policy' AND status IN ('executing','uncertain')"))
   for row in rows:
    body=self.store.open(row['id'],'autonomy',row['body']);who=body['who']
    if actual==body['after']:
     outcome='completed';db.execute("UPDATE autonomy_requests SET status='revoked' WHERE status IN ('pending','approved')")
    elif actual==body['before']:outcome='failed'
    else:
     self.stop_latch=True;self.stop_cutoff=time.time();outcome='uncertain'
    event=secrets.token_hex(16);payload=dict(event_id=event,agent=who['agent'],user=who['user'],event='tool_completed' if outcome=='completed' else 'tool_failed',request_id=row['id'],snapshot=row['snapshot'],linked_audit=body['audit_seq'])
    db.execute('UPDATE autonomy_requests SET status=? WHERE id=?',(outcome,row['id']));db.execute('INSERT INTO autonomy_audit(id,body) VALUES(?,?)',(event,self.store.seal(event,'autonomy_audit',payload)))
  if self.stop_latch:self.set_state(True)
 def current(self):
  try:
   with self.store.db() as db:row=db.execute("SELECT value FROM state WHERE key='autonomy.emergency'").fetchone()
   value=json.loads(row[0]);assert isinstance(value['stopped'],bool)
  except Exception:value=dict(stopped=True,generation=-1,cutoff=time.time(),unavailable=True)
  if self.stop_latch:value.update(stopped=True,cutoff=max(value.get('cutoff',0),self.stop_cutoff))
  return value
 def cfg(self):
  p=xiar_paths.policy_path();info=p.lstat()
  if not stat.S_ISREG(info.st_mode) or info.st_uid not in self.ROOT_UIDS or info.st_mode&0o022:raise ValueError('Unsafe Autonomy policy')
  return tomllib.loads(p.read_text())
 def publish(self,value):runtime.atomic_root(state_file(),json.dumps(value,allow_nan=False).encode(),0o644)
 def set_state(self,stopped):
  with self.emergency_lock,self.store.db() as db:
   old=json.loads(db.execute("SELECT value FROM state WHERE key='autonomy.emergency'").fetchone()[0]);value=dict(stopped=bool(stopped),generation=old['generation']+1,cutoff=time.time() if stopped else old['cutoff'])
   db.execute("UPDATE state SET value=? WHERE key='autonomy.emergency'",(json.dumps(value),));db.execute("UPDATE autonomy_requests SET status='revoked' WHERE status IN ('pending','approved')")
   if not stopped:db.execute("UPDATE autonomy_requests SET status='acknowledged' WHERE session='native-policy' AND status='uncertain'")
  self.publish(value)
  if not stopped:self.stop_latch=False
  return value
 def enqueue_audit(self,who,event,request='',snapshot=None,linked=None):
  id=secrets.token_hex(16);body=dict(event_id=id,agent=who['agent'],user=who['user'],event=event,request_id=request,snapshot=snapshot,linked_audit=linked)
  with self.store.db() as db:db.execute('INSERT INTO autonomy_audit(id,body) VALUES(?,?)',(id,self.store.seal(id,'autonomy_audit',body)))
  return id
 def send_audit(self,id):
  with self.store.db() as db:row=db.execute('SELECT * FROM autonomy_audit WHERE id=?',(id,)).fetchone()
  if not row or row['audited']:return dict(audited=True)
  key=memory.service_key(self.store.base);body=self.store.open(id,'autonomy_audit',row['body'])
  result=memory.request('/v1/autonomy/event',body,{'Authorization':'Bearer '+key})[1]
  with self.store.db() as db:db.execute('UPDATE autonomy_audit SET audited=1 WHERE id=?',(id,))
  return result
 def audit(self,who,event,request='',snapshot=None,linked=None):return self.send_audit(self.enqueue_audit(who,event,request,snapshot,linked))
 def flush(self):
  with self.store.db() as db:rows=list(db.execute('SELECT id FROM autonomy_audit WHERE audited=0'))
  for row in rows:self.send_audit(row['id'])
 def decision(self,who,tool,args,surface='hermes',approved=False):
  key=memory.service_key(self.store.base);stopped=self.current()['stopped']
  result=memory.request('/v1/autonomy/decide',dict(agent=who['agent'],user=who['user'],tool=tool,args=args,surface=surface,stopped=stopped,approved=approved,worker=bool(who.get('run_id'))),{'Authorization':'Bearer '+key})[1]
  # Full power (owner decision 2026-11-11): no tiers, ask-first or folder limits; the emergency stop still blocks changes.
  if isinstance(result,dict) and self.full_power(who) and not (stopped and result.get('mutating')):
   result=dict(result,allowed=True,forbidden=False,tier='free',approval_required=False,full_power=True);result.pop('reason',None)
  return result
 def full_power(self,who):
  try:
   if who.get('agent') in self.cfg().get('power',{}).get('full_agents',[]):return True
   return runtime.Profiles(runtime.ROOT).state()['agents'].get(who.get('agent'),{}).get('power')=='full'
  except Exception:return False
 def snapshot(self,request):
  q=subprocess.run(['snapper','-c','root','create','--type','pre','--description','autonomy:'+request,'--print-number'],capture_output=True,text=True,timeout=45,check=True)
  return int(q.stdout.strip())
 def stop(self,who):
  self.stop_latch=True;self.stop_cutoff=time.time()
  if hasattr(self,'drive'):
   try:self.drive.stop('Emergency stop')
   except Exception:pass
  try:value=self.set_state(True)
  except Exception:value=dict(self.current(),publication_failed=True)
  try:audit=self.enqueue_audit(who,'emergency_stop')
  except Exception:audit=None
  try:subprocess.run(['systemctl','stop','--no-block','xiar-hermes-subagents.target'],capture_output=True,timeout=15,check=False)
  except Exception:pass # journal/latch still closes admission; cancellation below still runs
  def cancel_all():
   try:
    with self.s.lifecycle:
     for row in self.store.rows(('queued',)+ACTIVE):
      if self.store.row(row['id'])['state'] not in TERMINAL:self.s.cancel(row)
   finally:
    try:self.send_audit(audit) if audit else None
    except Exception:self.store.audit('autonomy','canonical_audit_retry','Emergency stop persisted; canonical audit retry required')
  threading.Thread(target=cancel_all,daemon=True,name='xiar-emergency-stop').start();return value
 def request(self,body,who):
  action=body.get('action')
  if action=='status':return self.current()
  if action=='drive':return self.drive.request(who,body)
  if action=='screen':return self.screen_watch(who,body)
  if action=='inspect':
   from self_model import SelfModel
   return SelfModel(self).request(who,str(body.get('tool','')),body.get('args',{}))
  if action=='stop':
   if who.get('profile')!='owner' or who.get('run_id') or who.get('agent')!='xiar':raise ValueError('Global stop requires standing Xiar context')
   return self.stop(who)
  if action=='finish':
   request=str(body.get('request_id',''))
   with self.lock,self.store.db() as db:
    row=db.execute('SELECT * FROM autonomy_requests WHERE id=?',(request,)).fetchone()
    if not row or row['scope']!=who['profile'] or row['job']!=who.get('run_id',''):raise ValueError('Foreign Autonomy ticket')
    if row['status'] in ('completed','failed'):return dict(recorded=True)
    if row['status']!='executing':raise ValueError('Autonomy ticket is not open')
    sealed_body=self.store.open(request,'autonomy',row['body'])
    outcome='completed' if body.get('success') is True else 'failed';event=secrets.token_hex(16);payload=dict(event_id=event,agent=who['agent'],user=who['user'],event='tool_'+outcome,request_id=request,snapshot=row['snapshot'],linked_audit=sealed_body['audit_seq'])
    db.execute('UPDATE autonomy_requests SET status=? WHERE id=?',(outcome,request));db.execute('INSERT INTO autonomy_audit(id,body) VALUES(?,?)',(event,self.store.seal(event,'autonomy_audit',payload)))
   self.send_audit(event);return dict(recorded=True)
  if action!='begin':raise ValueError('Unknown runtime Autonomy operation')
  tool=body.get('tool');args=body.get('args');session=body.get('session','')
  if not isinstance(tool,str) or not isinstance(args,dict) or not isinstance(session,str) or len(session)>256:raise ValueError('Invalid Autonomy envelope')
  raw=json.dumps(dict(scope=who['profile'],job=who.get('run_id',''),session=session,tool=tool,args=args),sort_keys=True,allow_nan=False).encode()
  cfg=self.cfg()
  if len(raw)>cfg['policy']['max_args_bytes']:raise ValueError('Autonomy envelope exceeds configured bound')
  digest=hashlib.sha256(raw).hexdigest();now=time.time();generation=self.current()['generation']
  with self.lock:
   with self.store.db() as db:
    db.execute("UPDATE autonomy_requests SET status='expired' WHERE status IN ('pending','approved','denied') AND expires<?",(now,))
    old=db.execute("SELECT * FROM autonomy_requests WHERE digest=? AND status IN ('pending','approved','denied') AND generation=? ORDER BY created DESC LIMIT 1",(digest,generation)).fetchone()
   approved=bool(old and old['status']=='approved');decision=self.decision(who,tool,args,approved=approved)
   if decision.get('forbidden') or self.current()['stopped'] and decision.get('mutating'):return decision
   if not decision['allowed']:
    if old and old['status']=='denied':return dict(decision,request_id=old['id'],approval_required=False,reason='Owner denied this exact action; no repeated approval prompt during its validity window')
    if old:return dict(decision,request_id=old['id'],approval_required=True)
    with self.store.db() as db:count=db.execute("SELECT count(*) FROM autonomy_requests WHERE status IN ('pending','approved')").fetchone()[0]
    if count>=cfg['control']['max_pending']:return dict(decision,reason='Configured pending approval capacity reached')
    request=secrets.token_hex(16);sealed=self.store.seal(request,'autonomy',dict(args=args,who=who,audit_seq=decision['audit_seq']))
    with self.store.db() as db:db.execute('INSERT INTO autonomy_requests VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',(request,who['profile'],who.get('run_id',''),session,digest,tool,'pending',now,now+cfg['control']['approval_ttl_s'],generation,sealed,None))
    return dict(decision,request_id=request,approval_required=True)
   request=old['id'] if approved else secrets.token_hex(16);snapshot=None
   if decision['tier']=='logged_undoable':snapshot=self.snapshot(request)
   if approved:
    with self.store.db() as db:
     changed=db.execute("UPDATE autonomy_requests SET status='executing',snapshot=? WHERE id=? AND status='approved' AND generation=?",(snapshot,request,generation)).rowcount
     if changed!=1:raise ValueError('Approval consumed or revoked')
   else:
    sealed=self.store.seal(request,'autonomy',dict(args=args,who=who,audit_seq=decision['audit_seq']))
    with self.store.db() as db:db.execute('INSERT INTO autonomy_requests VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',(request,who['profile'],who.get('run_id',''),session,digest,tool,'executing',now,now+cfg['control']['approval_ttl_s'],generation,sealed,snapshot))
   if self.current()['generation']!=generation or self.current()['stopped'] and decision['mutating']:raise ValueError('Emergency state changed before execution')
   return dict(decision,request_id=request,snapshot=snapshot)
 ROOT_UIDS=xiar_paths.trusted_uids()
 SCREEN_CONSENT=xiar_paths.screen_consent_path()
 def screen_consent(self):
  try:
   path=self.SCREEN_CONSENT;info=path.lstat()
   if path.is_symlink() or info.st_uid not in self.ROOT_UIDS or info.st_mode&0o022:return {}
   value=json.loads(path.read_text(encoding='utf-8'));return value if isinstance(value,dict) else {}
  except (OSError,ValueError):return {}
 def screen_watch(self,who,body):
  """Xiar starts/ends a TIMED screen-watch session ONLY if the owner delegated that (Settings > Screen). Same single-use authority ticket as Drive; every start/stop is audited."""
  ticket,tool,args,generation=self.drive.ticket(who,body)
  if tool!='xiar_screen_watch':raise ValueError('Unknown screen operation')
  kind=args.get('action','status');old=self.screen_consent();delegate=old.get('delegate') if isinstance(old.get('delegate'),dict) else {}
  session=old.get('session') if isinstance(old.get('session'),dict) else None
  live=bool(session and isinstance(session.get('until'),(int,float)) and session['until']>time.time())
  status=dict(owner_on=old.get('enabled') is True,delegated=delegate.get('allowed') is True,max_minutes=delegate.get('max_minutes',0),session_active=live,session_until=session.get('until') if live else None,session_reason=session.get('reason') if live else None)
  if kind=='status':return status
  if kind not in ('start','stop'):raise ValueError('action must be start, stop or status')
  if not status['delegated']:raise ValueError('The owner has not allowed Xiar to start screen-watch sessions (Settings > Screen). Ask them to switch that on.')
  if self.current()['stopped'] or self.current()['generation']!=generation:raise ValueError('Emergency stop is on or changed; the screen monitor stays off')
  value=dict(old)
  if kind=='stop':value.pop('session',None)
  else:
   reason=str(args.get('reason','')).strip()[:200]
   if not reason:raise ValueError('A short reason (what Xiar is learning or doing) is required')
   minutes=args.get('minutes',10)
   if isinstance(minutes,bool) or not isinstance(minutes,(int,float)) or minutes<=0:raise ValueError('minutes must be a positive number')
   minutes=min(float(minutes),float(delegate.get('max_minutes',0)));now=time.time()
   value['session']=dict(until=now+minutes*60,reason=reason,by='xiar',started=now)
  with self.lock:runtime.atomic_root(self.SCREEN_CONSENT,json.dumps(value,allow_nan=False).encode(),0o644)
  # audited by the ticket itself: the Autonomy gate logged the request and the plugin's finish call logs the outcome
  return dict(status,session_active=kind=='start',session_until=value.get('session',{}).get('until'),session_reason=value.get('session',{}).get('reason'))
 def admin(self,command,args,user):
  who=dict(profile='owner',agent='xiar',user=tomllib.loads((runtime.ROOT/'config/hermes_adapter.toml').read_text())['identity']['end_user_id'])
  if command=='autonomy_replies_stop':return dict(cancelled=self.s.upstream.cancel_replies())
  if command=='autonomy_status':return dict(self.current(),pending=self.pending(),drive=self.drive.state(),poll_s=self.cfg()['control']['poll_s'],undoable=self.undoable())
  if command=='autonomy_permissions_get':
   with self.lock:
    self.cfg();raw=permissions.PATH.read_bytes();return permissions.public(tomllib.loads(raw.decode('utf-8')),raw)
  if command=='autonomy_permissions_set':
   with self.lock:
    cfg=self.cfg();raw=permissions.PATH.read_bytes();updated,desired=permissions.prepare(cfg,raw,args);generation=self.current()['generation']
    decision=self.decision(who,command,args,surface='native',approved=True)
    if not decision['allowed']:raise ValueError('Emergency stop or policy authority refuses permission changes')
    request=secrets.token_hex(16);snapshot=self.snapshot(request)
    # Preserve the complete before/after policy encrypted; never grant an Agent a policy write endpoint.
    body=dict(before=cfg,after=desired,who=who,audit_seq=decision['audit_seq'])
    with self.store.db() as db:db.execute('INSERT INTO autonomy_requests VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',(request,'owner','','native-policy',request,command,'executing',time.time(),time.time(),self.current()['generation'],self.store.seal(request,'autonomy',body),snapshot))
    if self.current()['stopped'] or self.current()['generation']!=generation:raise ValueError('Safety state changed before permission update')
    permissions.save(updated);event=secrets.token_hex(16)
    payload=dict(event_id=event,agent=who['agent'],user=who['user'],event='tool_completed',request_id=request,snapshot=snapshot,linked_audit=decision['audit_seq'])
    with self.store.db() as db:
     db.execute("UPDATE autonomy_requests SET status='completed' WHERE id=?",(request,));db.execute('INSERT INTO autonomy_audit(id,body) VALUES(?,?)',(event,self.store.seal(event,'autonomy_audit',payload)))
     db.execute("UPDATE autonomy_requests SET status='revoked' WHERE status IN ('pending','approved')")
    self.send_audit(event);return permissions.public(desired,updated)
  if command=='autonomy_stop':return self.stop(who)
  if command=='autonomy_screen_set':
   # The owner's consent to the screen monitor (Phase 8.4). Written ONLY here, root-owned: nothing running as the standing Xiar or an Agent can switch it on by itself.
   # The owner may also DELEGATE timed watch sessions to Xiar (delegate.allowed); the delegation, its length cap and an end-session switch are owner-only too.
   old=self.screen_consent();value=dict(old)
   if 'enabled' in args:
    enabled=args.get('enabled');interval=int(args.get('interval_s',old.get('interval_s',60)))
    if not isinstance(enabled,bool) or not 15<=interval<=3600:raise ValueError('enabled (true/false) and interval_s (15-3600) required')
    if enabled and self.current()['stopped']:raise ValueError('Emergency stop is on: resume before switching the screen monitor on')
    value.update(enabled=enabled,interval_s=interval,set_at=time.time(),by='owner')
    if not enabled:value.pop('session',None)  # switching it off ends any session Xiar started too
   if 'delegate' in args:
    allowed=args.get('delegate');cap=int(args.get('delegate_max_minutes',old.get('delegate',{}).get('max_minutes',30)))
    if not isinstance(allowed,bool) or not 1<=cap<=240:raise ValueError('delegate (true/false) and delegate_max_minutes (1-240) required')
    value['delegate']=dict(allowed=allowed,max_minutes=cap,set_at=time.time())
    if not allowed:value.pop('session',None)
   if args.get('end_session'):value.pop('session',None)
   value.setdefault('enabled',False);value.setdefault('interval_s',60)
   with self.lock:runtime.atomic_root(self.SCREEN_CONSENT,json.dumps(value,allow_nan=False).encode(),0o644)
   self.audit(who,'owner_approve' if (value['enabled'] or value.get('delegate',{}).get('allowed')) else 'owner_deny');return value
  if command=='autonomy_resume':
   self.audit(who,'owner_resume');return self.set_state(False)
  if command in ('autonomy_approve','autonomy_deny'):
   self.audit(who,'owner_approve' if command=='autonomy_approve' else 'owner_deny',str(args.get('request_id','')))
   request=str(args.get('request_id',''));now=time.time();generation=self.current()['generation']
   with self.lock,self.store.db() as db:
    row=db.execute('SELECT * FROM autonomy_requests WHERE id=?',(request,)).fetchone()
    if not row or row['status']!='pending' or row['expires']<now or row['generation']!=generation or self.current()['stopped']:raise ValueError('Approval expired, revoked, stopped or already answered')
    db.execute('UPDATE autonomy_requests SET status=? WHERE id=?',('approved' if command=='autonomy_approve' else 'denied',request))
   return dict(recorded=True)
  if command=='autonomy_effect':
   tool=str(args.get('tool',''));capture=effects.CAPTURE.get(tool)
   return dict(effects.describe(tool,self.cfg()['undo']['native_paths']),capture=dict(tool=capture[0],args=capture[1]) if capture else None)
  if command=='autonomy_native_gate':
   generation=self.current()['generation']
   result=self.decision(who,args['tool'],args.get('args',{}),surface='native',approved=True)
   if not result['allowed']:raise ValueError('Native operation refused by Rust policy')
   request=secrets.token_hex(16);paths=self.cfg()['undo']['native_paths'].get(args['tool'],[]);before=self.undo.capture(paths)
   state=args.get('before_state');inverse=effects.plan_inverse(args['tool'],args.get('args',{}),state if isinstance(state,dict) and len(json.dumps(state))<=262144 else None) if not paths else None
   snapshot=self.snapshot(request) if result['mutating'] and result['tier']!='free' else None;now=time.time()
   current=self.current()
   if result['mutating'] and (current['stopped'] or current['generation']!=generation):raise ValueError('Safety state changed before native dispatch')
   sealed=self.store.seal(request,'autonomy',dict(args=args.get('args',{}),who=who,audit_seq=result['audit_seq'],before=before,after=None,inverse=inverse,effect=effects.describe(args['tool'],self.cfg()['undo']['native_paths'])))
   with self.store.db() as db:db.execute('INSERT INTO autonomy_requests VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',(request,'owner','','native-owner',request,args['tool'],'executing',now,now,self.current()['generation'],sealed,snapshot))
   return dict(result,request_id=request,snapshot=snapshot)
  if command=='autonomy_native_finish':
   request=str(args.get('request_id',''))
   with self.lock:
    with self.store.db() as db:row=db.execute('SELECT * FROM autonomy_requests WHERE id=?',(request,)).fetchone()
    if not row or row['scope']!='owner' or row['session']!='native-owner':raise ValueError('Unknown native transaction')
    if row['status'] in ('completed','failed'):return dict(recorded=True)
    if row['status']!='executing':raise ValueError('Native transaction is not open')
    body=self.store.open(request,'autonomy',row['body']);body['after']=self.undo.capture([v['path'] for v in body['before']]);outcome='completed' if args.get('success') is True else 'failed'
    event=secrets.token_hex(16);payload=dict(event_id=event,agent=who['agent'],user=who['user'],event='tool_'+outcome,request_id=request,snapshot=row['snapshot'],linked_audit=body['audit_seq'])
    with self.store.db() as db:
     db.execute('UPDATE autonomy_requests SET status=?,body=? WHERE id=?',(outcome,self.store.seal(request,'autonomy',body),request));db.execute('INSERT INTO autonomy_audit(id,body) VALUES(?,?)',(event,self.store.seal(event,'autonomy_audit',payload)))
    self.send_audit(event);return dict(recorded=True)
  if command=='autonomy_undo':
   if self.current()['stopped']:raise ValueError('Emergency stop: resume explicitly before undoing a configuration change')
   request=str(args.get('request_id',''))
   with self.lock:
    with self.store.db() as db:row=db.execute('SELECT * FROM autonomy_requests WHERE id=?',(request,)).fetchone()
    if not row or row['scope']!='owner':raise ValueError('No owner configuration transaction')
    if row['status']=='undone':return dict(restored=True,already_recorded=True)
    if row['status'] not in ('completed','failed','undoing','uncertain'):raise ValueError('No restorable configuration transaction')
    body=self.store.open(request,'autonomy',row['body'])
    if not (body.get('before') and body.get('after')):
     if not body.get('inverse'):raise ValueError('No exact undo exists for this action: '+(body.get('effect') or effects.describe(row['tool'],self.cfg()['undo']['native_paths']))['how'])
     if row['status']!='undoing':
      body['undo_snapshot']=self.snapshot(request);body['undo_event']=secrets.token_hex(16);intent=secrets.token_hex(16)
      payload=dict(event_id=intent,agent=who['agent'],user=who['user'],event='undo',request_id=request,snapshot=row['snapshot'],linked_audit=body['audit_seq'])
      with self.store.db() as db:
       db.execute("UPDATE autonomy_requests SET status='undoing',body=? WHERE id=?",(self.store.seal(request,'autonomy',body),request));db.execute('INSERT INTO autonomy_audit(id,body) VALUES(?,?)',(intent,self.store.seal(intent,'autonomy_audit',payload)))
      self.send_audit(intent)
     return dict(inverse=body['inverse'],request_id=request,steps_run_by_settings=True)
    # A durable stable event identity precedes effects; completion/outbox commit together.
    if not body.get('undo_event'):
     body['undo_snapshot']=self.snapshot(request);body['undo_event']=secrets.token_hex(16);intent=secrets.token_hex(16)
     payload=dict(event_id=intent,agent=who['agent'],user=who['user'],event='undo',request_id=request,snapshot=row['snapshot'],linked_audit=body['audit_seq'])
     with self.store.db() as db:
      db.execute("UPDATE autonomy_requests SET status='undoing',body=? WHERE id=?",(self.store.seal(request,'autonomy',body),request));db.execute('INSERT INTO autonomy_audit(id,body) VALUES(?,?)',(intent,self.store.seal(intent,'autonomy_audit',payload)))
     self.send_audit(intent)
    result=self.undo.restore(body['before'],body['after']);event=body['undo_event'];body['undone_by']=event
    payload=dict(event_id=event,agent=who['agent'],user=who['user'],event='undo_completed',request_id=request,snapshot=row['snapshot'],linked_audit=body['audit_seq'])
    with self.store.db() as db:
     db.execute("UPDATE autonomy_requests SET status='undone',body=? WHERE id=?",(self.store.seal(request,'autonomy',body),request));db.execute('INSERT OR IGNORE INTO autonomy_audit(id,body) VALUES(?,?)',(event,self.store.seal(event,'autonomy_audit',payload)))
    self.send_audit(event);return result
  if command=='autonomy_undo_done':
   request=str(args.get('request_id',''))
   with self.lock:
    with self.store.db() as db:row=db.execute('SELECT * FROM autonomy_requests WHERE id=?',(request,)).fetchone()
    if not row or row['scope']!='owner':raise ValueError('No owner transaction')
    if row['status']=='undone':return dict(restored=True,already_recorded=True)
    if row['status']!='undoing':raise ValueError('The undo was not started')
    body=self.store.open(request,'autonomy',row['body']);event=body['undo_event'];body['undone_by']=event
    payload=dict(event_id=event,agent=who['agent'],user=who['user'],event='undo_completed',request_id=request,snapshot=row['snapshot'],linked_audit=body['audit_seq'])
    with self.store.db() as db:
     db.execute("UPDATE autonomy_requests SET status='undone',body=? WHERE id=?",(self.store.seal(request,'autonomy',body),request));db.execute('INSERT OR IGNORE INTO autonomy_audit(id,body) VALUES(?,?)',(event,self.store.seal(event,'autonomy_audit',payload)))
    self.send_audit(event);return dict(restored=True)
  raise ValueError('Unknown administrator Autonomy command')
 def undoable(self):
  with self.store.db() as db:rows=list(db.execute("SELECT * FROM autonomy_requests WHERE scope='owner' AND session='native-owner' AND status IN ('completed','failed','undoing','uncertain') ORDER BY created DESC LIMIT ?",(self.cfg()['control']['max_pending'],)))
  out=[]
  for row in rows:
   body=self.store.open(row['id'],'autonomy',row['body']);files=bool(body.get('before') and body.get('after'));inverse=bool(body.get('inverse'))
   if files or inverse:out.append(dict(request_id=row['id'],tool=row['tool'],reversible='files' if files else 'inverse',configuration_only=files,how=(body.get('effect') or {}).get('how',''),uncertain=row['status']=='uncertain'))
  return out
 def reconcile_orphans(self):
  """A transaction that began and never finished (the process died in between) is never left 'executing' forever and never assumed to have succeeded.
  Owner (native) transactions: compare the files with the saved before image: unchanged -> failed; changed -> uncertain, with the after image captured so the owner can undo it.
  Agent runtime tickets: marked uncertain (their effects cannot be known here). Nothing is replayed and no emergency stop is raised; every outcome is audited."""
  cutoff=time.time()-float(self.cfg()['control'].get('orphan_after_s',300));done=[]
  with self.lock:
   with self.store.db() as db:rows=list(db.execute("SELECT * FROM autonomy_requests WHERE status='executing' AND session!='native-policy' AND created<?",(cutoff,)))
   for row in rows:
    try:body=self.store.open(row['id'],'autonomy',row['body'])
    except Exception:continue
    who=body.get('who',dict(agent='xiar',user='owner'));before=body.get('before') or []
    if row['session']=='native-owner' and before:
     current=self.undo.capture([v['path'] for v in before]);outcome,note=effects.classify_orphan(before,current,Undo.same)
     if outcome=='uncertain':body['after']=current
    else:outcome,note='uncertain','The action began and never recorded its end; its effect cannot be known. Review it by hand.'
    body['reconciled']=dict(outcome=outcome,note=note,at=time.time());event=secrets.token_hex(16)
    payload=dict(event_id=event,agent=who['agent'],user=who['user'],event='tool_failed',request_id=row['id'],snapshot=row['snapshot'],linked_audit=body.get('audit_seq'))
    with self.store.db() as db:
     db.execute('UPDATE autonomy_requests SET status=?,body=? WHERE id=? AND status=?',(outcome,self.store.seal(row['id'],'autonomy',body),row['id'],'executing'));db.execute('INSERT INTO autonomy_audit(id,body) VALUES(?,?)',(event,self.store.seal(event,'autonomy_audit',payload)))
    done.append((row['id'],outcome))
  return done
 def pending(self):
  def redact(value):
   if isinstance(value,dict):return {k:'[redacted]' if any(s in k.lower() for s in ('password','secret','token','authorization','credential','api_key','otp','cvv','card_number')) else redact(v) for k,v in value.items()}
   if isinstance(value,list):return [redact(v) for v in value]
   return value
  now=time.time();generation=self.current()['generation'];out=[]
  with self.store.db() as db:rows=list(db.execute("SELECT * FROM autonomy_requests WHERE status='pending' AND expires>? AND generation=? ORDER BY created LIMIT ?",(now,generation,self.cfg()['control']['max_pending'])))
  for row in rows:
   body=self.store.open(row['id'],'autonomy',row['body']);out.append(dict(request_id=row['id'],profile=row['scope'],run_id=row['job'],tool=row['tool'],args=redact(body['args']),expires=row['expires']))
  return out
