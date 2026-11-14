# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Sealed cold storage for idle private Agents ("start when used").
An Agent that has been stopped for a while has its mutable data (sessions, state database, logs, workspace, skills it made, memories, cron, kanban) packed,
compressed and sealed (AES-256-GCM, a key derived per Agent from a root-only key) into one file under the root-owned control directory, and the originals are
removed. The configuration, adapter, tokens and service never move. The next start restores everything before the service starts.
Root only. Nothing here edits Hermes; the Agent's own Linux user is not running while this happens, and the originals are removed only after the sealed copy
was read back and its hash matched."""
import hashlib,hmac,json,os,pathlib,secrets,shutil,stat,struct,subprocess,tarfile,tempfile,threading,time
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
import runtime
from xiar_hermes.profiles import Profiles

BASE=runtime.CONTROL/'cold'
KEY=runtime.CONTROL/'cold.key'
CHUNK=1024*1024
MAGIC=b'XCS1'
MAX_BYTES=2*1024**3
LOCK=threading.RLock()

def _key(profile):
    runtime.private(runtime.CONTROL)
    try:
        fd=os.open(KEY,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
        with os.fdopen(fd,'wb') as out:out.write(secrets.token_bytes(32));out.flush();os.fsync(out.fileno())
    except FileExistsError:pass
    info=KEY.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid!=0 or info.st_mode&0o077 or info.st_size!=32:raise ValueError('Unsafe cold storage key')
    return hmac.new(KEY.read_bytes(),b'xiar-cold|'+profile.encode(),hashlib.sha256).digest()

def _home(agent,profile):
    p=Profiles(runtime.ROOT);home=p.base/'isolated'/runtime.slug(agent)/runtime.slug(profile)
    if home.is_symlink() or not home.is_dir():raise ValueError('Unknown Agent home')
    return home

def _active(profile):
    return subprocess.run(['systemctl','is-active','--quiet',runtime.profile_unit(profile)],timeout=20).returncode==0

def _present(home):return [n for n in runtime.MUTABLE if os.path.lexists(home/n)]

def _size(path):
    total=0
    for root,dirs,files in os.walk(path,followlinks=False):
        for f in files:
            try:total+=os.lstat(os.path.join(root,f)).st_size
            except OSError:pass
    return total

def _seal(tar_path,out_path,key,profile):
    aead=AESGCM(key);prefix=secrets.token_bytes(4);digest=hashlib.sha256();index=0
    with open(tar_path,'rb') as src,open(out_path,'wb') as out:
        out.write(MAGIC+prefix)
        block=src.read(CHUNK)
        while True:
            following=src.read(CHUNK);last=not following
            digest.update(block)
            nonce=prefix+struct.pack('>Q',index);out.write(struct.pack('>I',len(block)))
            out.write(aead.encrypt(nonce,block,(profile+'|'+str(index)+'|'+('1' if last else '0')).encode()))
            index+=1
            if last:break
            block=following
        out.flush();os.fsync(out.fileno())
    return digest.hexdigest()

def _open(sealed,out_tar,key,profile):
    aead=AESGCM(key);digest=hashlib.sha256()
    with open(sealed,'rb') as src,open(out_tar,'wb') as out:
        if src.read(4)!=MAGIC:raise ValueError('Not a sealed cold archive')
        prefix=src.read(4);index=0
        while True:
            head=src.read(4)
            if len(head)!=4:raise ValueError('Truncated cold archive')
            plain=struct.unpack('>I',head)[0];body=src.read(plain+16)
            if len(body)!=plain+16:raise ValueError('Truncated cold archive')
            last=src.read(1)==b''
            if not last:src.seek(-1,os.SEEK_CUR)
            block=aead.decrypt(prefix+struct.pack('>Q',index),body,(profile+'|'+str(index)+'|'+('1' if last else '0')).encode())
            digest.update(block);out.write(block);index+=1
            if last:break
        out.flush();os.fsync(out.fileno())
    return digest.hexdigest()

def status(profile):
    meta=BASE/(runtime.slug(profile)+'.json')
    if not meta.is_file() or meta.is_symlink():return dict(cold=False)
    value=json.loads(meta.read_text());return dict(cold=True,**{k:value[k] for k in ('stored_at','files','bytes_before','bytes_after')})

def store(agent,profile):
    """Pack and seal an idle Agent's data. Refuses a running Agent, links and special files. Returns the sizes."""
    profile=runtime.slug(profile)
    with LOCK:
        if _active(profile):raise ValueError('The Agent is running; cold storage only takes a stopped Agent')
        if status(profile)['cold']:raise ValueError('Already in cold storage')
        home=_home(agent,profile);names=_present(home)
        if not names:return dict(stored=False,reason='Nothing to store')
        before=sum(_size(home/n) if (home/n).is_dir() and not (home/n).is_symlink() else (home/n).lstat().st_size for n in names)
        if before>MAX_BYTES:raise ValueError('Too large for cold storage')
        for n in names:
            for root,dirs,files in os.walk(home/n,followlinks=False):
                for entry in list(dirs)+files:
                    mode=os.lstat(os.path.join(root,entry)).st_mode
                    if not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):raise ValueError('The Agent data contains a link or special file; it was left as it is')
            mode=os.lstat(home/n).st_mode
            if not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):raise ValueError('The Agent data contains a link or special file; it was left as it is')
        BASE.mkdir(parents=True,exist_ok=True,mode=0o700);os.chmod(BASE,0o700)
        key=_key(profile)
        with tempfile.TemporaryDirectory(dir=BASE) as scratch:
            tar_path=os.path.join(scratch,'a.tar')
            with tarfile.open(tar_path,'w:xz',preset=3) as tar:   # compressed here, then sealed: encrypted data cannot be compressed afterwards
                for n in names:tar.add(home/n,arcname=n,recursive=True,filter=lambda t:(setattr(t,'uid',0),setattr(t,'gid',0),setattr(t,'uname',''),setattr(t,'gname',''),t)[-1])
            sealed_tmp=os.path.join(scratch,'a.sealed');digest=_seal(tar_path,sealed_tmp,key,profile)
            # read it back before anything is removed
            check=os.path.join(scratch,'check.tar')
            if _open(sealed_tmp,check,key,profile)!=digest:raise ValueError('Sealed copy did not verify; nothing was removed')
            os.chmod(sealed_tmp,0o600);final=BASE/(profile+'.sealed');os.replace(sealed_tmp,final)
            after=final.stat().st_size
        meta=dict(profile=profile,agent=agent,stored_at=int(time.time()),files=names,bytes_before=before,bytes_after=after,tar_sha256=digest)
        runtime.atomic_root(BASE/(profile+'.json'),json.dumps(meta).encode())
        for n in names:
            path=home/n
            if path.is_dir() and not path.is_symlink():shutil.rmtree(path)
            else:path.unlink()
        return dict(stored=True,bytes_before=before,bytes_after=after,files=names)

def restore(agent,profile):
    """Put the data back, owned by the Agent's user, before its service starts. No-op when the Agent is not in cold storage."""
    profile=runtime.slug(profile)
    with LOCK:
        info=status(profile)
        if not info['cold']:return False
        if _active(profile):raise ValueError('The Agent is already running')
        home=_home(agent,profile);meta=json.loads((BASE/(profile+'.json')).read_text());acc=runtime.records()[agent]
        for n in meta['files']:
            if os.path.lexists(home/n):raise ValueError('Cold data conflicts with new data in the Agent home; nothing was restored')
        key=_key(profile)
        with tempfile.TemporaryDirectory(dir=BASE) as scratch:
            tar_path=os.path.join(scratch,'a.tar')
            if _open(BASE/(profile+'.sealed'),tar_path,key,profile)!=meta['tar_sha256']:raise ValueError('Cold archive hash mismatch; nothing was restored')
            with tarfile.open(tar_path,'r') as tar:
                members=tar.getmembers()
                for member in members:
                    top=pathlib.PurePosixPath(member.name).parts
                    if not top or member.name.startswith('/') or '..' in top or top[0] not in meta['files'] or not (member.isreg() or member.isdir()):raise ValueError('Unsafe entry in the cold archive; nothing was restored')
                tar.extractall(home,members=members,filter='data')
        for n in meta['files']:
            for root,dirs,files in os.walk(home/n,followlinks=False):
                for entry in [root]+[os.path.join(root,x) for x in dirs+files]:os.lchown(entry,acc['uid'],acc['gid'])
            os.lchown(home/n,acc['uid'],acc['gid'])
        (BASE/(profile+'.sealed')).unlink();(BASE/(profile+'.json')).unlink()
        return True

def _newest(home,names):
    newest=0
    for n in names:
        newest=max(newest,os.lstat(home/n).st_mtime)
        if (home/n).is_dir() and not (home/n).is_symlink():
            for root,dirs,files in os.walk(home/n,followlinks=False):
                for entry in dirs+files:newest=max(newest,os.lstat(os.path.join(root,entry)).st_mtime)
    return newest

def sweep(idle_seconds):
    """Store every 'start when used' Agent that is stopped and untouched for idle_seconds. Called from the broker's background thread."""
    out=[]
    state=runtime.state()
    for row in state['scopes'].values():
        agent=row['agent'];cfg=state['agents'].get(agent,{})
        if cfg.get('run_mode')!='on_demand' or not cfg.get('linux_user'):continue
        try:
            profile=row['profile']
            if status(profile)['cold'] or _active(profile):continue
            home=_home(agent,profile);names=_present(home)
            newest=_newest(home,names)
            if names and time.time()-newest>=idle_seconds:out.append((profile,store(agent,profile)))
        except Exception as error:out.append((row.get('profile','?'),dict(error=str(error)[:120])))
    return out
