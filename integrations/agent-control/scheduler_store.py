# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Root-only durable work journal. Task/stream/result contents are AEAD-encrypted, never logged."""
from pathlib import Path
import contextlib,hashlib,json,os,sqlite3,stat,threading,time
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
ACTIVE=('admitting','launching','running','yielding','waiting_children','resuming','cancelling','releasing')
TERMINAL=('completed','failed','cancelled','killed_budget')
class Store:
 def __init__(self,directory):
  if sqlite3.sqlite_version_info<(3,54,1):raise ValueError('Patched external SQLite is required before the WAL journal opens')
  self.base=Path(directory)
  if self.base.is_symlink():raise ValueError('Unsafe scheduler state root')
  self.base.mkdir(parents=True,exist_ok=True,mode=0o700)
  if self.base.stat().st_uid!=0 or not self.base.is_dir():raise ValueError('Scheduler root must be owned by root')
  os.chmod(self.base,0o700)
  key=self.base/'queue.key'
  try:
   fd=os.open(key,os.O_CREAT|os.O_EXCL|os.O_WRONLY|os.O_NOFOLLOW,0o600)
   with os.fdopen(fd,'wb') as out:out.write(AESGCM.generate_key(bit_length=256));out.flush();os.fsync(out.fileno())
  except FileExistsError:pass
  st=key.stat()
  if key.is_symlink() or not stat.S_ISREG(st.st_mode) or st.st_uid!=os.getuid() or st.st_mode&0o077:raise ValueError('Unsafe scheduler encryption capability')
  self.aead=AESGCM(key.read_bytes());self.lock=threading.RLock();self.path=self.base/'queue.sqlite3'
  if self.path.is_symlink():raise ValueError('Unsafe scheduler journal')
  if self.path.exists():
   info=self.path.stat()
   if not stat.S_ISREG(info.st_mode) or info.st_uid!=0 or info.st_mode&0o077:raise ValueError('Unsafe existing journal ownership')
  with self.db() as db:
   db.executescript("""CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,scope TEXT NOT NULL,parent TEXT NOT NULL DEFAULT '',priority INTEGER NOT NULL,state TEXT NOT NULL,reason TEXT NOT NULL,created REAL NOT NULL,updated REAL NOT NULL,payload BLOB NOT NULL,digest TEXT NOT NULL,template TEXT NOT NULL DEFAULT '',weight TEXT NOT NULL DEFAULT 'heavy',placement BLOB,generation TEXT NOT NULL DEFAULT '',result BLOB,released INTEGER NOT NULL DEFAULT 0,archived INTEGER NOT NULL DEFAULT 0); CREATE INDEX IF NOT EXISTS jobs_ready ON jobs(state,priority,created); CREATE INDEX IF NOT EXISTS jobs_scope ON jobs(scope); CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY AUTOINCREMENT,job TEXT NOT NULL,body BLOB NOT NULL); CREATE TABLE IF NOT EXISTS state(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS controls(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE NOT NULL,job TEXT NOT NULL,body BLOB NOT NULL,delivered INTEGER NOT NULL DEFAULT 0); CREATE INDEX IF NOT EXISTS controls_job ON controls(job,seq);""")
  os.chmod(self.path,0o600)
 @contextlib.contextmanager
 def db(self):
  with self.lock:
   db=sqlite3.connect(self.path,timeout=30);db.row_factory=sqlite3.Row;db.execute('PRAGMA journal_mode=WAL');db.execute('PRAGMA synchronous=FULL');db.execute('PRAGMA foreign_keys=ON')
   try:
    with db:yield db
   finally:db.close()
 def seal(self,job,field,value):
  nonce=os.urandom(12);raw=json.dumps(value,ensure_ascii=False,allow_nan=False,separators=(',',':')).encode();return nonce+self.aead.encrypt(nonce,raw,(job+'|'+field).encode())
 def open(self,job,field,value):return json.loads(self.aead.decrypt(value[:12],value[12:],(job+'|'+field).encode()))
 def row(self,job):
  with self.db() as db:
   row=db.execute('SELECT * FROM jobs WHERE id=?',(job,)).fetchone();return dict(row) if row else None
 def rows(self,states=None,scope=None):
  sql='SELECT * FROM jobs';where=[];args=[]
  if states:where.append('state IN ('+','.join('?' for _ in states)+')');args.extend(states)
  if scope is not None:where.append('scope=?');args.append(scope)
  if where:sql+=' WHERE '+' AND '.join(where)
  with self.db() as db:return [dict(v) for v in db.execute(sql+' ORDER BY priority,created,id',args)]
 def submit(self,job,scope,parent,priority,payload,max_queue):
  digest=hashlib.sha256(json.dumps(payload,sort_keys=True,allow_nan=False).encode()).hexdigest();now=time.time()
  with self.db() as db:
   old=db.execute('SELECT scope,parent,digest FROM jobs WHERE id=?',(job,)).fetchone()
   if old:
    if old['scope']!=scope or old['parent']!=parent or old['digest']!=digest:raise ValueError('Conflicting durable run identity')
    return False
   if db.execute("SELECT count(*) FROM jobs WHERE state='queued'").fetchone()[0]>=max_queue:raise ValueError('Configured queue limit reached')
   db.execute('INSERT INTO jobs(id,scope,parent,priority,state,reason,created,updated,payload,digest) VALUES(?,?,?,?,?,?,?,?,?,?)',(job,scope,parent,priority,'queued','Durably queued',now,now,self.seal(job,'payload',payload),digest));return True
 def update(self,job,**values):
  allowed={'state','reason','template','weight','placement','generation','result','released','archived'}
  if not values or set(values)-allowed:raise ValueError('Unknown durable job field')
  for field in ('placement','result'):
   if field in values and values[field] is not None:values[field]=self.seal(job,field,values[field])
  values['updated']=time.time()
  with self.db() as db:db.execute('UPDATE jobs SET '+','.join(k+'=?' for k in values)+' WHERE id=?',list(values.values())+[job])
 def transition(self,job,expected,**values):
  allowed={'state','reason','template','weight','generation','released'}
  if not values or set(values)-allowed:raise ValueError('Unknown atomic transition field')
  values['updated']=time.time()
  with self.db() as db:
   q='UPDATE jobs SET '+','.join(k+'=?' for k in values)+' WHERE id=? AND state IN ('+','.join('?' for _ in expected)+')'
   return db.execute(q,list(values.values())+[job]+list(expected)).rowcount==1
 def event(self,job,value,limit):
  with self.db() as db:
   db.execute('INSERT INTO events(job,body) VALUES(?,?)',(job,self.seal(job,'event',value)))
   db.execute('DELETE FROM events WHERE job=? AND id NOT IN(SELECT id FROM events WHERE job=? ORDER BY id DESC LIMIT ?)',(job,job,limit))
 def events(self,job,after,max_bytes):
  out=[];used=0
  with self.db() as db:
   for value in db.execute('SELECT id,body FROM events WHERE job=? AND id>? ORDER BY id',(job,after)):
    row={'cursor':value['id'],'event':self.open(job,'event',value['body'])};size=len(json.dumps(row,ensure_ascii=True).encode())
    if used+size>max_bytes:break
    out.append(row);used+=size
  return out
 def audit(self,job,event,reason):
  # Operational metadata only. No task text, session content, credentials or raw exceptions.
  body=dict(time=time.time(),run_id=job,event=event,reason=str(reason)[:300]);raw=(json.dumps(body,allow_nan=False)+'\n').encode()
  with self.lock:
   fd=os.open(self.base/'audit.jsonl',os.O_WRONLY|os.O_APPEND|os.O_CREAT|os.O_NOFOLLOW,0o600)
   try:os.write(fd,raw);os.fsync(fd)
   finally:os.close(fd)

 def steer(self,job,control,text,limit):
  with self.db() as db:
   old=db.execute('SELECT job,body FROM controls WHERE id=?',(control,)).fetchone()
   if old:
    if old['job']!=job or self.open(job,'control:'+control,old['body'])!=text:raise ValueError('Control identity reused')
    return True
   row=db.execute('SELECT state FROM jobs WHERE id=?',(job,)).fetchone()
   closed=db.execute('SELECT value FROM state WHERE key=?',('controls_closed:'+job,)).fetchone()
   if not row or row['state'] not in ('queued','admitting','launching','running') or closed:return False
   if db.execute('SELECT count(*) FROM controls WHERE job=?',(job,)).fetchone()[0]>=limit:raise ValueError('Steering queue bound exceeded')
   db.execute('INSERT INTO controls(id,job,body) VALUES(?,?,?)',(control,job,self.seal(job,'control:'+control,text)));return True
 def controls(self,job,after=0,delivered=(),close=False):
  with self.db() as db:
   for control in delivered:db.execute('UPDATE controls SET delivered=1 WHERE id=? AND job=?',(control,job))
   if close:db.execute('INSERT OR IGNORE INTO state(key,value) VALUES(?,?)',('controls_closed:'+job,'1'))
   return [dict(seq=v['seq'],id=v['id'],text=self.open(job,'control:'+v['id'],v['body']),delivered=bool(v['delivered'])) for v in db.execute('SELECT * FROM controls WHERE job=? AND seq>? ORDER BY seq',(job,int(after)))]

 def control_summary(self,job):
  with self.db() as db:return [dict(id=v['id'],delivered=bool(v['delivered'])) for v in db.execute('SELECT id,delivered FROM controls WHERE job=? ORDER BY seq',(job,))]
