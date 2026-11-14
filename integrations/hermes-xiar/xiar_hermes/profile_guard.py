# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Folder limits for native file tools. Not an OS sandbox for shell execution.
The approved shared-UID default is not hostile-code isolation. Dedicated Linux
users remain a separate optional provisioner, never silently claimed equivalent.
"""
from pathlib import Path
import re
READ={'read_file','search_files','file_info','glob','list_directory'}
WRITE={'write_file','patch','apply_patch','edit_file','delete_file','move_file','copy_file'}
FIELDS={'path','file_path','filepath','directory','cwd','working_dir','source','destination','src','dst','target','paths','files'}
def limits(cfg,tool,args):
    base=cfg.get('profile.home')
    if not base or tool not in READ|WRITE:return None
    home=Path(base).resolve();workspace=home/'workspace';read=[home,*[Path(p).resolve() for p in cfg.get('profile.read_only_roots',[])]]
    values=[]
    def collect(value):
        if isinstance(value,str):values.append(value)
        elif isinstance(value,list):
            for item in value:collect(item)
    for key,value in (args or {}).items():
        if key in FIELDS:collect(value)
    if tool in ('patch','apply_patch'):
        for key in ('patch','patch_text','input'):
            for match in re.finditer(r'^\*\*\* (?:Update|Add|Delete) File: (.+)$',str((args or {}).get(key,'')),re.MULTILINE):values.append(match.group(1))
    if not values:return {'action':'block','message':'File tool did not provide a verifiable path inside this private Agent home'}
    for value in values:
        try:
            path=Path(value).expanduser();path=(path if path.is_absolute() else workspace/path).resolve()
            if path.parent==home and path.name in {'.env','memory-api-token','adapter.json','.xiar-identity.json','config.yaml','profile.yaml'}:
                raise ValueError()
            allowed=read if tool in READ else [home]
            if not any(path.is_relative_to(root) for root in allowed):raise ValueError()
        except (OSError,RuntimeError,ValueError):return {'action':'block','message':'Path is outside this Agent private home and permitted read-only shared libraries'}
    return None
