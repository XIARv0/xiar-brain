# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Standing Xiar's approved, scoped Drive. No screenshot capture, user key logging or replay.
Kernel Requisite/PartOf gates close the stop-versus-late-unit-start race; the visible chrome
lease is mandatory before input. Exact actions remain subject to the original tier broker.
"""
from pathlib import Path
import json,os,re,secrets,stat,subprocess,threading,time
import runtime
class Drive:
 def __init__(self,a):
  self.a=a;self.lock=threading.RLock();self.guard=threading.RLock();self.epoch=0;self.chrome_seen=0;self.chrome_session='';self.active_session=''
  with a.store.db() as db:db.execute('CREATE TABLE IF NOT EXISTS autonomy_drive_effects(ticket TEXT PRIMARY KEY,status TEXT NOT NULL,result BLOB)')
  self.stop('Broker restarted; Drive sessions never auto-resume')
  def watch():
   while True:
    delay=1.0 # fail-closed retry only if the root policy cannot be read
    try:
     delay=float(self.a.cfg()['control']['poll_s'])
     if not 0<delay<=60:raise ValueError('Invalid Drive safety cadence')
     s=self.state()
     if s.get('stop_retry_required'):self.stop('Retrying an incomplete kernel stop')
     elif s.get('active'):
      if self.a.current()['stopped'] or self.cfg()['allowed_window_classes'] and s['window_class'] not in self.cfg()['allowed_window_classes'] or time.time()>=s['deadline'] or time.monotonic()-self.chrome_seen>self.cfg()['indicator_lease_s']:self.stop('Stop, session cap or visible-indicator lease ended')
      else:
       window=self.probe()
       with self.guard:
        live=self.state()
        if live.get('active') and live.get('session_id')==s['session_id'] and (window.get('class')!=s['window_class'] or window.get('address')!=s['window_address']):self.stop('Active window changed or became untrusted/sensitive')
    except Exception:
     try:self.stop('Drive authority unavailable')
     except Exception:pass # retry on every safety tick; never lose the watcher on a storage failure
    finally:threading.Event().wait(delay if 0<delay<=60 else 1.0)
  threading.Thread(target=watch,daemon=True,name='xiar-drive-safety').start()
 def cfg(self):return self.a.cfg()['drive']
 def state(self):
  with self.a.store.db() as db:row=db.execute("SELECT value FROM state WHERE key='autonomy.drive'").fetchone()
  return json.loads(row[0]) if row else dict(active=False)
 @staticmethod
 def target(session):
  if not re.fullmatch('[a-f0-9]{32}',session):raise ValueError('Invalid Drive session')
  return 'xiar-drive-session@'+session+'.target'
 def publish(self,value):
  with self.a.store.db() as db:db.execute("INSERT INTO state(key,value) VALUES('autonomy.drive',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",(json.dumps(value,allow_nan=False),))
  path=Path(self.cfg()['indicator_file']);path.parent.mkdir(parents=True,exist_ok=True,mode=0o755);info=path.parent.lstat()
  if not stat.S_ISDIR(info.st_mode) or info.st_uid!=0:raise ValueError('Unsafe public indicator directory')
  os.chmod(path.parent,0o755)
  # Only cadence/session metadata is public, never injection text or a window title.
  runtime.atomic_root(path,json.dumps(dict(value,poll_ms=int(self.a.cfg()['control']['poll_s']*1000))).encode(),0o644)
 @staticmethod
 def freeze_input(unit):
  # Fixed root system-slice cgroup only, never a supplied path or a recycled process ID.
  from autonomy_undo import directory
  try:parent=directory(Path('/sys/fs/cgroup/system.slice')/unit)
  except FileNotFoundError:return
  try:
   for name in ('cgroup.freeze','cgroup.kill'):
    try:fd=os.open(name,os.O_WRONLY|os.O_NOFOLLOW,dir_fd=parent)
    except FileNotFoundError:continue
    try:os.write(fd,b'1')
    finally:os.close(fd)
  finally:os.close(parent)
 def stop(self,reason):
  # Never wait for the input action lock. A stopped kernel target rejects even a late start.
  with self.guard:
   self.epoch+=1
   try:s=self.state()
   except Exception:s={}
   sessions={v for v in (self.active_session,s.get('session_id','')) if re.fullmatch('[a-f0-9]{32}',v)}
   failures=[]
   for unit in ['xiar-ydotool.service']+['xiar-drive-'+v+'.service' for v in sessions]:
    try:self.freeze_input(unit)
    except Exception:pass # synchronous systemd termination below remains authoritative
   for unit in [self.target(v) for v in sessions]+['xiar-ydotool.service']:
    try:
     q=subprocess.run(['systemctl','stop',unit],capture_output=True,timeout=10,check=False)
     if q.returncode:failures.append(unit)
    except Exception:failures.append(unit)
   if not failures:self.active_session=''
   self.chrome_session='';self.chrome_seen=0
   self.publish(dict(active=bool(failures and sessions),session_id=s.get('session_id','') or self.active_session,reason=reason,stop_retry_required=bool(failures)))
   if failures:raise ValueError('Kernel Drive stop needs retry')
   return self.state()
 def start_chrome(self,peer):
  if peer[1]!=runtime.OWNER.pw_uid:raise ValueError('Only the desktop owner can start the fixed chrome')
  subprocess.run(['systemctl','start',self.cfg()['chrome_unit']],capture_output=True,timeout=self.cfg()['chrome_start_timeout_s'],check=True)
  return dict(started=True)
 def chrome(self,peer,body):
  if peer[1]!=runtime.OWNER.pw_uid:raise ValueError('Desktop owner UID required')
  status=(Path('/proc')/str(peer[0])/'status').read_text();match=re.search(r'^PPid:\s*(\d+)$',status,re.M)
  if not match:raise ValueError('Desktop chrome lineage unavailable')
  # Merely being a child of a root-owned executable is NOT authority: any Agent can run qs.
  # Require the live MainPID and non-delegated cgroup of the fixed root system service.
  unit=self.cfg()['chrome_unit']
  main=subprocess.check_output(['systemctl','show',unit,'--property=MainPID','--value'],timeout=5,text=True).strip()
  group=(Path('/proc')/match[1]/'cgroup').read_text().splitlines()
  if main!=match[1] or '0::/system.slice/'+unit not in group:raise ValueError('Trusted root desktop chrome unit required')
  executable=Path('/proc')/match[1]/'exe';expected=Path(self.cfg()['indicator_parent_executable']);info=expected.stat()
  if executable.resolve()!=expected.resolve() or info.st_uid!=0 or info.st_mode&0o022:raise ValueError('Root-installed compositor chrome required')
  if body.get('action')=='stop':
   import tomllib
   identity=tomllib.loads((runtime.ROOT/'config/hermes_adapter.toml').read_text())['identity']
   return self.a.stop(dict(profile='owner',agent=identity['agent_id'],user=identity['end_user_id']))
  session=body.get('session_id','')
  if body.get('action')!='heartbeat' or not re.fullmatch('[a-f0-9]{32}',session):raise ValueError('Bounded visible indicator heartbeat required')
  with self.guard:
   current=self.state()
   if current.get('active') and current.get('session_id')==session:self.chrome_seen=time.monotonic();self.chrome_session=session
  return dict(recorded=True)
 def unit(self,session,request):
  cfg=self.cfg();helper=Path(cfg['helper']);info=helper.lstat()
  if not stat.S_ISREG(info.st_mode) or info.st_uid!=0 or info.st_mode&0o022:raise ValueError('Reviewed root-owned Drive helper required')
  injecting=request.get('mode')=='action';root_input=request.get('action',{}).get('kind')=='click'
  uid=0 if root_input else runtime.OWNER.pw_uid;gid=0 if root_input else runtime.OWNER.pw_gid
  argv=['systemd-run','--quiet','--wait','--pipe','--collect','--service-type=exec','--unit=xiar-drive-'+session+'.service','--uid='+str(uid),'--gid='+str(gid),'--property=NoNewPrivileges=yes','--property=DevicePolicy=closed','--property=ProtectControlGroups=yes','--property=ProtectKernelTunables=yes','--property=ProtectKernelModules=yes','--property=KillMode=control-group','--property=KillSignal=SIGKILL','--property=RuntimeMaxSec='+str(cfg['command_timeout_s'])+'s','--property=TimeoutStopSec=0','--property=ProtectSystem=strict','--property=ProtectHome=read-only','--property=RestrictAddressFamilies=AF_UNIX','--property=MemoryMax='+str(cfg['helper_memory_mb'])+'M','--property=TasksMax='+str(cfg['helper_max_tasks'])]
  if injecting:
   target=self.target(session)
   argv+=['--property=Requisite='+target,'--property=After='+target,'--property=PartOf='+target]
  argv+=['--',str(helper)]
  q=subprocess.run(argv,input=json.dumps(request,allow_nan=False),text=True,capture_output=True,timeout=float(cfg['command_timeout_s'])+10)
  if q.returncode or len(q.stdout)>self.a.cfg()['policy']['max_args_bytes']:raise ValueError('Drive helper refused; no unsafe fallback')
  value=json.loads(q.stdout)
  if value.get('error') or value.get('blocked'):raise ValueError('Sensitive, unavailable or changed desktop window; Drive paused')
  return value
 def probe(self):return self.unit(secrets.token_hex(16),dict(mode='probe'))
 def ticket(self,who,body):
  if who.get('profile')!='owner' or who.get('run_id') or who.get('agent')!='xiar':raise ValueError('Only standing Xiar may drive the owner desktop')
  ticket=str(body.get('request_id',''));tool=body.get('tool');args=body.get('args',{});current=self.a.current()
  with self.a.store.db() as db:row=db.execute('SELECT * FROM autonomy_requests WHERE id=?',(ticket,)).fetchone()
  if not row or row['scope']!='owner' or row['job'] or row['tool']!=tool or row['status']!='executing' or row['generation']!=current['generation']:raise ValueError('Current exact-action authority required')
  sealed=self.a.store.open(ticket,'autonomy',row['body'])
  if sealed['args']!=args:raise ValueError('Final Drive arguments differ from authorized arguments')
  return ticket,tool,args,current['generation']
 def check(self,generation,epoch):
  current=self.a.current()
  if current['stopped'] or current['generation']!=generation or self.epoch!=epoch or self.state().get('stop_retry_required') or not self.cfg()['enabled']:raise ValueError('Drive authority changed/stopped')
 def request(self,who,body):
  ticket,tool,args,generation=self.ticket(who,body)
  if tool=='xiar_drive_status':return dict(session=self.state(),window=self.probe(),screen_consent_implied=False)
  if tool=='xiar_drive_end':return self.stop('Xiar ended its Drive session')
  if tool not in ('xiar_drive_start','xiar_drive_action'):raise ValueError('Unknown Drive operation')
  with self.lock:
   with self.guard:epoch=self.epoch;self.check(generation,epoch)
   with self.a.store.db() as db:
    old=db.execute('SELECT * FROM autonomy_drive_effects WHERE ticket=?',(ticket,)).fetchone()
    if old:
     if old['result']:return self.a.store.open(ticket,'drive_result',old['result'])
     raise ValueError('Drive outcome uncertain; input is never automatically replayed')
    db.execute("INSERT INTO autonomy_drive_effects(ticket,status) VALUES(?,'started')",(ticket,))
   try:
    if tool=='xiar_drive_start':
     window=self.probe()
     if not window.get('class') or window['class']!=args.get('window_class') or window['address']!=args.get('window_address'):raise ValueError('Session scope must match the current trusted window')
     if self.cfg()['allowed_window_classes'] and window['class'] not in self.cfg()['allowed_window_classes']:raise ValueError('Window outside the owner-configured scope; only the owner may change that allowlist')
     snapshot=self.a.snapshot(ticket)
     with self.a.store.db() as db:db.execute('UPDATE autonomy_requests SET snapshot=? WHERE id=?',(snapshot,ticket))
     with self.guard:
      self.check(generation,epoch);self.stop('Replaced by a new explicitly approved session');session=secrets.token_hex(16);self.active_session=session
      self.check(generation,self.epoch)
      subprocess.run(['systemctl','start',self.target(session)],capture_output=True,timeout=10,check=True)
      subprocess.run(['systemctl','start','xiar-ydotool.service'],capture_output=True,timeout=20,check=True)
      subprocess.run(['systemctl','is-active','--quiet','xiar-ydotool.service'],capture_output=True,timeout=5,check=True)
      self.check(generation,self.epoch);now=time.time();self.chrome_seen=time.monotonic();self.chrome_session=''
      self.publish(dict(active=True,session_id=session,window_class=window['class'],window_address=window['address'],deadline=now+self.cfg()['max_session_s'],recent_actions=[],reason='Awaiting visible chrome acknowledgement'))
      result=dict(session_id=session,awaiting_indicator=True,deadline=now+self.cfg()['max_session_s'],snapshot=snapshot)
    else:
     with self.guard:
      self.check(generation,epoch);s=self.state();session=args.get('session_id')
      if not s.get('active') or s['session_id']!=session or self.chrome_session!=session or time.monotonic()-self.chrome_seen>self.cfg()['indicator_lease_s'] or time.time()>=s['deadline']:raise ValueError('Active visible, scoped Drive session required')
      recent=[at for at in s['recent_actions'] if at>time.time()-60]
      if len(recent)>=self.cfg()['max_actions_per_minute']:raise ValueError('Drive action rate exceeded')
      recent.append(time.time());s['recent_actions']=recent;self.publish(s)
     # A stop after this point removes the target: Requisite prevents a late injection unit.
     result=self.unit(session,dict(mode='action',generation=generation,window_class=s['window_class'],window_address=s['window_address'],action=args.get('action',{})))
     if not result.get('window_unchanged'):self.stop('Window changed after action; explicit new scope required')
   except Exception:
    try:self.stop('Drive failed, stopped, unavailable or outside the approved window')
    finally:
     with self.a.store.db() as db:db.execute("UPDATE autonomy_drive_effects SET status='uncertain' WHERE ticket=?",(ticket,))
    raise
   with self.a.store.db() as db:db.execute("UPDATE autonomy_drive_effects SET status='finished',result=? WHERE ticket=?",(self.a.store.seal(ticket,'drive_result',result),ticket))
   return result
