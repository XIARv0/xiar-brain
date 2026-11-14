# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Bounded configuration before-images. Caller stores them only through the scheduler AEAD seal.
No raw live database rollback, credential file copy, full-root rollback or arbitrary path restoration.
"""
from pathlib import Path
import base64,hashlib,os,secrets,stat
import runtime
def directory(path):
 """Anchor every absolute ancestor by descriptor, rejecting symlinks and lexical traversal."""
 path=Path(path)
 if not path.is_absolute() or any(p in ('.','..') for p in path.parts):raise ValueError('Absolute anchored directory required')
 fd=os.open('/',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
 try:
  for part in path.parts[1:]:
   new=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd);os.close(fd);fd=new
  return fd
 except BaseException:os.close(fd);raise
class Undo:
 def __init__(self,cfg):self.cfg=cfg
 def parent(self,relative):
  cfg=self.cfg()['undo']
  if relative not in cfg['allowed_paths']:raise ValueError('Undo target is not an explicitly configured non-secret file')
  parts=relative.split('/')
  if any(p in ('','.','..') for p in parts):raise ValueError('Unsafe undo path')
  fd=directory(runtime.ROOT)
  try:
   for part in parts[:-1]:
    new=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd);os.close(fd);fd=new
   return fd,parts[-1]
  except BaseException:os.close(fd);raise
 def capture(self,paths):
  cfg=self.cfg()['undo']
  if len(paths)>cfg['max_files'] or len(set(paths))!=len(paths):raise ValueError('Undo file list exceeds configured bound')
  out=[];total=0
  for relative in paths:
   directory,name=self.parent(relative)
   try:
    try:fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=directory)
    except FileNotFoundError:out.append(dict(path=relative,exists=False));continue
    try:
     before=os.fstat(fd)
     if not stat.S_ISREG(before.st_mode) or before.st_nlink!=1 or before.st_size>cfg['max_file_bytes']:raise ValueError('Unsafe or oversized undo source')
     chunks=[];size=0
     while True:
      block=os.read(fd,min(65536,cfg['max_file_bytes']+1-size))
      if not block:break
      chunks.append(block);size+=len(block)
      if size>cfg['max_file_bytes']:raise ValueError('Undo source grew beyond configured bound')
     after=os.fstat(fd)
     if (before.st_ino,before.st_size,before.st_mtime_ns,before.st_ctime_ns)!=(after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns):raise ValueError('Undo source changed while captured')
     raw=b''.join(chunks);total+=len(raw)
     if total>cfg['max_total_bytes']:raise ValueError('Undo aggregate exceeds configured bound')
     out.append(dict(path=relative,exists=True,content=base64.b64encode(raw).decode(),digest=hashlib.sha256(raw).hexdigest(),mode=stat.S_IMODE(before.st_mode),uid=before.st_uid,gid=before.st_gid))
    finally:os.close(fd)
   finally:os.close(directory)
  return out
 @staticmethod
 def same(a,b):return a['exists']==b['exists'] and (not a['exists'] or all(a[k]==b[k] for k in ('digest','mode','uid','gid')))
 def restore(self,before,after):
  if [v['path'] for v in before]!=[v['path'] for v in after]:raise ValueError('Undo path sets differ')
  current=self.capture([v['path'] for v in before])
  # Preflight every file before any restoration; never overwrite unrelated later edits.
  if any(not self.same(now,old) and not self.same(now,new) for now,old,new in zip(current,before,after)):raise ValueError('Undo conflict: a target changed after this action')
  for now,old,new in zip(current,before,after):
   now=self.capture([old['path']])[0]
   if not self.same(now,old) and not self.same(now,new):raise ValueError('Undo conflict: target changed since preflight')
   if self.same(now,old):continue # a prior interrupted undo already restored this file
   directory,name=self.parent(old['path']);temporary='.xiar-undo-'+secrets.token_hex(16)
   try:
    if not old['exists']:os.unlink(name,dir_fd=directory);os.fsync(directory);continue
    fd=os.open(temporary,os.O_CREAT|os.O_EXCL|os.O_WRONLY|os.O_NOFOLLOW,0o600,dir_fd=directory)
    try:
     raw=base64.b64decode(old['content'],validate=True)
     if hashlib.sha256(raw).hexdigest()!=old['digest']:raise ValueError('Sealed undo image integrity invalid')
     with os.fdopen(fd,'wb',closefd=False) as stream:stream.write(raw);stream.flush()
     os.fchown(fd,old['uid'],old['gid']);os.fchmod(fd,old['mode']);os.fsync(fd)
    finally:os.close(fd)
    os.replace(temporary,name,src_dir_fd=directory,dst_dir_fd=directory);os.fsync(directory)
   finally:
    try:os.unlink(temporary,dir_fd=directory)
    except FileNotFoundError:pass
    os.close(directory)
  final=self.capture([v['path'] for v in before])
  if any(not self.same(a,b) for a,b in zip(final,before)):raise ValueError('Undo verification did not reach recorded before state')
  return dict(restored_files=len(final),configuration_only=True)
