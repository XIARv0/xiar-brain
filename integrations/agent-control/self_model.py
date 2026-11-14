# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Standing Xiar bounded self-model. No argv, environment, credentials or lesson payloads."""
from pathlib import Path
import datetime,json,os,re,stat,subprocess,tomllib,urllib.parse
from zoneinfo import ZoneInfo
from autonomy_undo import directory
import runtime
import paths as xiar_paths

def read_regular(path,limit):
 parent=directory(path.parent)
 try:fd=os.open(path.name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=parent)
 finally:os.close(parent)
 try:
  before=os.fstat(fd)
  if not stat.S_ISREG(before.st_mode) or before.st_nlink!=1 or before.st_size>limit:raise ValueError('Unsafe or oversized public configuration')
  chunks=[];size=0
  while True:
   part=os.read(fd,min(65536,limit+1-size))
   if not part:break
   chunks.append(part);size+=len(part)
   if size>limit:raise ValueError('Public configuration grew beyond its bound')
  after=os.fstat(fd)
  if (before.st_ino,before.st_size,before.st_mtime_ns,before.st_ctime_ns)!=(after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns):raise ValueError('Public configuration changed while read')
  return b''.join(chunks).decode('utf-8')
 finally:os.close(fd)

def redact(value):
 if isinstance(value,dict):return {k:'[redacted]' if any(s in k.lower() for s in ('password','secret','token','credential','authorization','api_key','master_key')) else redact(v) for k,v in value.items()}
 if isinstance(value,list):return [redact(v) for v in value]
 if isinstance(value,str) and '://' in value:
  parsed=urllib.parse.urlsplit(value)
  if parsed.hostname:return parsed.scheme+'://'+parsed.hostname+(':'+str(parsed.port) if parsed.port else '')
  return '[redacted URL]'
 return value

def owner_path(cfg,key,default=''):
 raw=str(cfg.get(key,default) or '').strip()
 if not raw:return None
 return Path(runtime.OWNER.pw_dir)/raw

def zone_name(cfg):
 return xiar_paths.timezone_name(cfg)

def iso(stamp,zone):return datetime.datetime.fromtimestamp(stamp,ZoneInfo(zone)).isoformat()

LEVEL=re.compile(r'\b(DEBUG|INFO|WARNING|ERROR|CRITICAL)\b')
SLUG=re.compile(r'[a-z0-9][a-z0-9-]{0,62}')

class SelfModel:
 def __init__(self,coordinator):self.a=coordinator
 def request(self,who,tool,args):
  if who.get('profile')!='owner' or who.get('run_id') or who.get('agent')!='xiar':raise ValueError('Standing Xiar self-model only; private profiles/workers cannot enumerate owner state')
  cfg=self.a.cfg()['inspect'];limit=int(cfg['max_entries'])
  if tool=='inspect_hardware':return self.a.s.laya('/v1/resources',{})
  if tool=='inspect_code_tree':
   base=args.get('root',cfg['code_roots'][0])
   if base not in cfg['code_roots']:raise ValueError('Choose an explicitly configured public code root')
   out=[];stack=[(directory(runtime.ROOT/base),Path(base))];examined=0;truncated=False
   try:
    while stack and examined<limit:
     fd,relative=stack.pop()
     try:
      with os.scandir(fd) as entries:
       for entry in entries:
        if examined>=limit:truncated=True;break
        examined+=1
        if entry.name.startswith('.') or entry.name in cfg['skip_dirs']:continue
        info=entry.stat(follow_symlinks=False)
        if stat.S_ISDIR(info.st_mode):
         child=os.open(entry.name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd);stack.append((child,relative/entry.name))
        elif stat.S_ISREG(info.st_mode):out.append(dict(path=str(relative/entry.name),bytes=info.st_size))
     finally:os.close(fd)
    truncated=truncated or bool(stack)
   finally:
    for fd,_ in stack:os.close(fd)
   return dict(files=out,truncated=truncated,metadata_only=True,examined=examined)
  if tool=='inspect_config':
   relative=args.get('file',cfg['config_files'][0])
   if relative=='hermes-profile':
    # the standing Xiar's Hermes profile: provider/model, plugins and the tool allowlists (platform_toolsets). Credentials and URL private parts are redacted; read-only.
    try:import yaml
    except ImportError:raise ValueError('YAML reader unavailable')
    path=owner_path(cfg,'hermes_config')
    if path is None:raise ValueError('Hermes profile path is not configured')
    return dict(file='hermes-profile',configuration=redact(yaml.safe_load(read_regular(path,int(cfg['max_config_bytes'])))),credential_values=False)
   if relative not in cfg['config_files']:raise ValueError('Configuration file not granted to self-model')
   return dict(file=relative,configuration=redact(tomllib.loads(read_regular(runtime.ROOT/relative,int(cfg['max_config_bytes'])))),credential_values=False)
  if tool=='inspect_processes':
   q=subprocess.run(['ps','-eo','pid=,ppid=,uid=,comm='],capture_output=True,text=True,timeout=15,check=True);rows=[]
   for line in q.stdout.splitlines():
    values=line.split(None,3)
    if len(values)==4 and int(values[2])==runtime.OWNER.pw_uid:rows.append(dict(pid=int(values[0]),parent=int(values[1]),uid=int(values[2]),program=values[3]))
    if len(rows)>=limit:break
   return dict(processes=rows,argv=False,environment=False,metadata_only=True)
  if tool=='inspect_sessions':
   # the standing Xiar's Hermes session records: counts, sizes and times only. No names, no content.
   path=owner_path(cfg,'session_dir')
   if path is None:return dict(sessions=0,total_bytes=0,newest=None,oldest=None,recent=[],metadata_only=True,names=False,content=False,configured=False)
   fd=directory(path);rows=[]
   try:
    with os.scandir(fd) as entries:
     for entry in entries:
      info=entry.stat(follow_symlinks=False)
      if stat.S_ISREG(info.st_mode):rows.append((info.st_mtime,info.st_size))
      if len(rows)>=limit*16:break
   finally:os.close(fd)
   rows.sort(reverse=True);zone=zone_name(cfg)
   return dict(sessions=len(rows),total_bytes=sum(r[1] for r in rows),newest=iso(rows[0][0],zone) if rows else None,oldest=iso(rows[-1][0],zone) if rows else None,recent=[dict(at=iso(r[0],zone),bytes=r[1]) for r in rows[:limit]],metadata_only=True,names=False,content=False)
  if tool=='inspect_agent_logs':
   # the Hermes agent / error / gateway logs: size, last write and how many lines of each level in the recent tail. No line text.
   path=owner_path(cfg,'hermes_log_dir')
   if path is None:return dict(logs=[],metadata_only=True,line_text=False,configured=False)
   fd=directory(path);out=[];zone=zone_name(cfg)
   try:
    for name in cfg.get('hermes_logs',['agent.log','errors.log','gateway.log']):
     try:log=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=fd)
     except FileNotFoundError:continue
     try:
      info=os.fstat(log)
      if not stat.S_ISREG(info.st_mode) or info.st_nlink!=1:continue
      offset=max(0,info.st_size-int(cfg['max_log_bytes']));os.lseek(log,offset,os.SEEK_SET);raw=os.read(log,int(cfg['max_log_bytes']))
     finally:os.close(log)
     levels={};lines=raw.decode('utf-8','replace').splitlines()
     if offset:lines=lines[1:]
     for line in lines:
      found=LEVEL.search(line[:80])
      if found:levels[found.group(1)]=levels.get(found.group(1),0)+1
     out.append(dict(log=name,bytes=info.st_size,modified=iso(info.st_mtime,zone),tail_lines=len(lines),levels=levels))
   finally:os.close(fd)
   return dict(logs=out,metadata_only=True,line_text=False)
  if tool=='inspect_memory_catalogue':
   # what kinds of memory exist for ONE business and how many entries: counts only. Never entries, lessons, trainer data or Vault values.
   business=str(args.get('business',''))
   if not SLUG.fullmatch(business):raise ValueError('Choose one business by its normalized id')
   import scheduler_memory
   return scheduler_memory.catalogue(who,business,limit)
  if tool=='inspect_logs':
   p=self.a.store.base/'audit.jsonl';parent=directory(p.parent)
   try:
    try:fd=os.open(p.name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=parent)
    except FileNotFoundError:return dict(events=[],metadata_only=True)
   finally:os.close(parent)
   try:
    info=os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_uid!=0 or info.st_nlink!=1:raise ValueError('Unsafe operational log')
    offset=max(0,info.st_size-int(cfg['max_log_bytes']));os.lseek(fd,offset,os.SEEK_SET);raw=os.read(fd,int(cfg['max_log_bytes']))
   finally:os.close(fd)
   lines=raw.splitlines()
   if offset:lines=lines[1:]
   events=[]
   for line in lines[-limit:]:
    try:
     item=json.loads(line);events.append(dict(at=datetime.datetime.fromtimestamp(item['time'],ZoneInfo(zone_name(cfg))).isoformat(),event=item['event'],run_id=item['run_id']))
    except (ValueError,KeyError,TypeError):continue
   return dict(events=events,timezone=zone_name(cfg),metadata_only=True,raw_payloads=False)
  raise ValueError('Unknown self-model read')
