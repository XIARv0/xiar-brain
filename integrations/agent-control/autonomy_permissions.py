# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Owner-only policy editing. Root caller verifies the actual administrator session first.
No credential, memory identity, trainer grant, implicit tool or executable path may be introduced.
"""
from pathlib import Path
import copy,hashlib,json,os,re,tomllib
from autonomy_undo import directory
import paths as xiar_paths
import runtime
FIELDS={'policy':('maintained_roots','free_read_tools','free_write_tools'),'drive':('enabled','max_actions_per_minute','max_session_s','allowed_window_classes','sensitive_window_classes'),'control':('max_pending','approval_ttl_s')}
PATH=xiar_paths.policy_path()
def public(cfg,raw):return dict(revision=hashlib.sha256(raw).hexdigest(),values={section:{key:cfg[section][key] for key in keys} for section,keys in FIELDS.items()})
def prepare(cfg,raw,args):
 if set(args)!={'revision','values'} or args['revision']!=hashlib.sha256(raw).hexdigest():raise ValueError('Policy changed; reload before editing permissions')
 values=args['values']
 if not isinstance(values,dict) or set(values)!=set(FIELDS):raise ValueError('Explicit complete permission sections required')
 desired=copy.deepcopy(cfg)
 for section,keys in FIELDS.items():
  if not isinstance(values[section],dict) or set(values[section])!=set(keys):raise ValueError('Unexpected permission field')
  for key in keys:
   value=values[section][key]
   if key=='enabled':
    if not isinstance(value,bool):raise ValueError('Drive enabled must be boolean')
   elif key in ('max_actions_per_minute','max_session_s','max_pending','approval_ttl_s'):
    if isinstance(value,bool) or not isinstance(value,int) or not 0<value<2**31:raise ValueError('Permission limit must be a positive bounded integer')
   else:
    if not isinstance(value,list) or len(value)>cfg['control']['max_pending'] or len(set(v for v in value if isinstance(v,str)))!=len(value):raise ValueError('Unique bounded text grant list required')
    for entry in value:
     if not isinstance(entry,str) or not entry or len(entry)>4096 or any(ord(c)<32 for c in entry):raise ValueError('Invalid permission grant')
     if key=='maintained_roots':
      p=Path(entry)
      if not p.is_absolute() or '..' in p.parts or not p.is_dir() or p.resolve()!=p:raise ValueError('Maintained roots must be existing absolute non-symlink directories')
     elif key in ('free_read_tools','free_write_tools'):
      if not re.fullmatch(r'[A-Za-z0-9_.:-]{1,128}',entry):raise ValueError('Invalid tool identifier')
      if entry in cfg['policy']['read_tools']+cfg['policy']['write_tools']:raise ValueError('Filesystem tools retain their target/credential/trainer guards; use maintained roots instead')
      if entry.startswith('autonomy_') or entry in ('xiar_drive_start','xiar_drive_action'):raise ValueError('Permission controls and scoped Drive cannot be widened through a generic Free-tool list')
    if key=='free_write_tools' and 'xiar_record_lesson' not in value:raise ValueError('Learning must not be changed into an action approval queue')
   desired[section][key]=value
 # Only replace existing whitelisted scalar/list lines, preserving every other setting/comment.
 lines=[];section='';seen=set()
 for line in raw.decode('utf-8').splitlines():
  header=re.fullmatch(r'\[([^\]]+)\]\s*(?:#.*)?',line.strip())
  if header:section=header[1]
  field=re.match(r'^([a-z_]+)\s*=',line)
  if section in FIELDS and field and field[1] in FIELDS[section]:
   key=field[1];line=key+' = '+json.dumps(desired[section][key],ensure_ascii=True,allow_nan=False);seen.add((section,key))
  lines.append(line)
 if seen!={(section,key) for section,keys in FIELDS.items() for key in keys}:raise ValueError('Policy schema changed; no partial rewrite permitted')
 updated=('\n'.join(lines)+'\n').encode()
 if tomllib.loads(updated.decode())!=desired:raise ValueError('Policy round-trip mismatch')
 return updated,desired

def save(updated):
 runtime.atomic_root(PATH,updated,0o644)
 fd=directory(PATH.parent)
 try:os.fsync(fd)
 finally:os.close(fd)
