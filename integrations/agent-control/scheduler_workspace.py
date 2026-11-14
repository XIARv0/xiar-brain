# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Bounded regular-file handoff. All privileged traversal is fd-anchored and refuses symlinks."""
from pathlib import Path
import hashlib,json,os,stat
import runtime

def directory(path,create=False,uid=0,gid=0):
 path=Path(path)
 if not path.is_absolute():raise ValueError('Absolute workspace path required')
 fd=os.open('/',os.O_RDONLY|os.O_DIRECTORY)
 try:
  for part in path.parts[1:]:
   if part in ('','.','..'):raise ValueError('Unsafe workspace component')
   if create:
    try:os.mkdir(part,0o700,dir_fd=fd);os.chown(part,uid,gid,dir_fd=fd,follow_symlinks=False)
    except FileExistsError:pass
   child=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd);os.close(fd);fd=child
  return fd
 except BaseException:os.close(fd);raise

def copy_tree(source,dest,cfg,uid,gid):
 # Source is a quiescent private worker tree, or one explicitly bound parent workspace.
 # No hidden credentials/config, sockets, links, devices, proc files or hardlinks travel.
 budget=[0,0];maximum=int(cfg['workspace_max_bytes']);count=int(cfg['workspace_max_files']);filemax=int(cfg['workspace_file_max_bytes'])
 def walk(src,dst,depth=0):
  if depth>64:raise ValueError('Workspace nesting exceeds bound')
  for name in sorted(os.listdir(src)):
   budget[1]+=1
   if budget[1]>count:raise ValueError('Workspace object count exceeds bound')
   if name.startswith('.') or name in ('node_modules','venv','__pycache__'):continue
   item=os.stat(name,dir_fd=src,follow_symlinks=False)
   if stat.S_ISLNK(item.st_mode):raise ValueError('Workspace symlink refused')
   if stat.S_ISDIR(item.st_mode):
    try:os.mkdir(name,0o700,dir_fd=dst)
    except FileExistsError:pass
    a=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=src);b=None
    try:
     b=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=dst);os.fchown(b,uid,gid);walk(a,b,depth+1)
    finally:
     os.close(a)
     if b is not None:os.close(b)
   elif stat.S_ISREG(item.st_mode):
    if item.st_nlink!=1 or item.st_size>filemax:raise ValueError('Workspace file outside bounds')
    budget[0]+=item.st_size
    if budget[0]>maximum or budget[1]>count:raise ValueError('Workspace aggregate bound exceeded')
    a=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=src);b=None
    try:
     check=os.fstat(a)
     if not stat.S_ISREG(check.st_mode) or check.st_nlink!=1 or check.st_size!=item.st_size or check.st_ino!=item.st_ino or check.st_dev!=item.st_dev:raise ValueError('Workspace changed during handoff')
     try:b=os.open(name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600,dir_fd=dst)
     except FileExistsError:
      old=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=dst)
      try:
       meta=os.fstat(old)
       if not stat.S_ISREG(meta.st_mode) or meta.st_nlink!=1 or meta.st_size!=item.st_size:raise ValueError('Refusing to overwrite changed artifact')
       left=hashlib.sha256();right=hashlib.sha256()
       for handle,digest in ((a,left),(old,right)):
        size=0
        while True:
         chunk=os.read(handle,min(1048576,filemax-size+1))
         if not chunk:break
         size+=len(chunk)
         if size>filemax:raise ValueError('Artifact comparison exceeds bound')
         digest.update(chunk)
       if left.digest()!=right.digest():raise ValueError('Refusing to overwrite changed artifact')
      finally:os.close(old)
      continue
     os.fchown(b,uid,gid)
     copied=0
     while True:
      chunk=os.read(a,min(1048576,filemax-copied+1))
      if not chunk:break
      copied+=len(chunk)
      if copied>item.st_size:raise ValueError('Growing workspace file refused')
      view=memoryview(chunk)
      while view:view=view[os.write(b,view):]
     end=os.fstat(a)
     if copied!=item.st_size or end.st_mtime_ns!=check.st_mtime_ns or end.st_ctime_ns!=check.st_ctime_ns or end.st_size!=check.st_size:raise ValueError('Workspace changed during copy')
     os.fsync(b)
    finally:
     os.close(a)
     if b is not None:os.close(b)
   else:raise ValueError('Nonregular workspace object refused')
 a=directory(source);b=None
 try:
  b=directory(dest);walk(a,b)
 finally:
  os.close(a)
  if b is not None:os.close(b)
 return dict(bytes=budget[0],files=budget[1])

def scope_workspace(who,cfg):
 if who['profile']=='owner':return runtime.ROOT/cfg['workspace_dir']/'owner'
 p=runtime.Profiles(runtime.ROOT);row=p.state()['scopes'].get(who['profile'])
 if not row or any(row[k]!=who[k] for k in ('agent','user','business')):raise ValueError('Workspace scope changed')
 return p.base/'isolated'/who['agent']/who['profile']/'workspace'

def seed(home,payload,cfg,uid,gid,parent_workspace=None):
 source=str(payload.get('workspace') or '');allowed=scope_workspace(payload['principal'],cfg)
 info=dict(seeded=False,reason='Fresh private workspace; native source outside the bound workspace is not copied')
 if source:
  selected=Path(source)
  alias=Path('/var/lib/xiar-agent/workspace')
  if parent_workspace is not None and selected.is_absolute() and selected.is_relative_to(alias):
   selected=Path(parent_workspace)/selected.relative_to(alias)
   return dict(seeded=True,source='Scoped parent workspace',**copy_tree(selected,home/'workspace',cfg,uid,gid))
  if selected.is_absolute() and selected.is_relative_to(allowed):
   # Never resolve arbitrary links and then adopt their destination as a granted path.
   info=dict(seeded=True,source=str(selected),**copy_tree(selected,home/'workspace',cfg,uid,gid))
 return info

def export(home,job,who,cfg,uid,gid,parent_workspace=None):
 parent=Path(parent_workspace) if parent_workspace is not None else scope_workspace(who,cfg);fd=directory(parent,True,uid,gid)
 try:
  try:os.mkdir('subagents',0o700,dir_fd=fd);os.chown('subagents',uid,gid,dir_fd=fd,follow_symlinks=False)
  except FileExistsError:pass
  sub=os.open('subagents',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
  try:
   try:os.mkdir(job,0o700,dir_fd=sub);os.chown(job,uid,gid,dir_fd=sub,follow_symlinks=False)
   except FileExistsError:pass
  finally:os.close(sub)
 finally:os.close(fd)
 target=parent/'subagents'/job
 return dict(path=str(target),**copy_tree(home/'workspace',target,cfg,uid,gid))
