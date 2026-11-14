# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""t208u: state for the minimized apps strip in the root-defined compositor chrome (os/quickshell/config/xiar/shell.qml).
Prints one JSON line: the strip settings, the XIAR desktop window (t212u: shown as a Desktop button while apps are open) from the layered desktop config ([minimized_strip], same layers as os/xiar-desktop/avatar_config.py)
and the windows on the "minimized" side workspace (where the title bar's minimize button puts them). Read only; no credentials."""
from pathlib import Path
import json,os,re,subprocess,tomllib
DEFAULT={'always_show':False,'position':'bottom','positions':['bottom','top','left','right'],'desktop_profile':'xiar-kiosk'}
def expanded(value):return Path(os.path.expandvars(os.path.expanduser(value)))
def merge(a,b):
 for k,v in b.items():
  if isinstance(v,dict) and isinstance(a.get(k),dict):merge(a[k],v)
  else:a[k]=v
def settings():
 try:
  cfg=tomllib.loads((Path(os.environ['XIAR_ROOT'])/'config/desktop.toml').read_text());paths=cfg['configuration']
  for path in [expanded(paths['defaults']),expanded(paths['system']),*sorted(expanded(paths['system_dir']).glob('*.toml')),expanded(paths['user']),expanded(paths['session'])]:
   try:
    if path.is_file() and not path.is_symlink():merge(cfg,tomllib.loads(path.read_text()))
   except (OSError,tomllib.TOMLDecodeError):pass
  m={**DEFAULT,**cfg.get('minimized_strip',{})}
 except (OSError,KeyError,tomllib.TOMLDecodeError):m=dict(DEFAULT)
 if not isinstance(m['desktop_profile'],str) or not m['desktop_profile']:m['desktop_profile']=DEFAULT['desktop_profile']
 if not isinstance(m['always_show'],bool):m['always_show']=False
 if m['position'] not in ('bottom','top','left','right'):m['position']='bottom'
 return {'always_show':m['always_show'],'position':m['position'],'desktop_profile':m['desktop_profile']}
def hyprctl(*args):
 env=dict(os.environ)
 if not env.get('HYPRLAND_INSTANCE_SIGNATURE'):
  try:env['HYPRLAND_INSTANCE_SIGNATURE']=max((Path(env.get('XDG_RUNTIME_DIR','/run/user/%d'%os.getuid()))/'hypr').iterdir(),key=lambda p:p.stat().st_mtime).name
  except (OSError,ValueError):pass
 return subprocess.run(['hyprctl',*args],capture_output=True,timeout=5,check=True,env=env)
def clients():
 try:c=json.loads(hyprctl('-j','clients').stdout)
 except (OSError,subprocess.SubprocessError,ValueError):return []
 return [x for x in c if isinstance(x,dict) and x.get('mapped',True)] if isinstance(c,list) else []
def is_desktop(c,profile):
 """The desktop is the browser window whose process was started with --user-data-dir=.../<desktop_profile> (its title changes as pages change)."""
 # Chromium rewrites its process title as one space-separated string, so read the flag with a pattern rather than by NUL-separated arguments.
 try:raw=Path('/proc/%d/cmdline'%int(c.get('pid',0))).read_bytes()
 except (OSError,ValueError):return False
 return any(d.rstrip(b'/').split(b'/')[-1]==profile.encode() for d in re.findall(rb'--user-data-dir=([^\0 ]+)',raw))
def state():
 s=settings();desktop=None;apps=0;out=[]
 for c in clients():
  ws=str((c.get('workspace') or {}).get('name',''))
  if desktop is None and is_desktop(c,s['desktop_profile']):
   desktop={'address':str(c.get('address','')),'minimized':ws=='special:minimized'};continue
  apps+=1
  # t214u: every open app is listed (a taskbar); minimized ones are on the "minimized" side workspace. Other side workspaces (scratchpads) are left out.
  if ws.startswith('special:') and ws!='special:minimized':continue
  out.append({'address':str(c.get('address','')),'title':str(c.get('title') or c.get('initialTitle') or ''),'class':str(c.get('class') or ''),'minimized':ws=='special:minimized'})
 return {'always_show':s['always_show'],'position':s['position'],'desktop':desktop,'apps_open':apps>0,'windows':out}
def park_desktop():
 """t212u: called by the desktop launcher after a restart: if apps are open, put the desktop on the strip (minimized) instead of on top of them.
 Exit 0 once the desktop window exists, 1 while it is not there yet."""
 s=state();d=s['desktop']
 if not d:return 1
 if s['apps_open'] and not d['minimized']:
  try:hyprctl('dispatch','movetoworkspacesilent','special:minimized,address:'+d['address'])
  except (OSError,subprocess.SubprocessError):pass
 return 0
if __name__=='__main__':
 import sys
 if sys.argv[1:]==['park-desktop']:raise SystemExit(park_desktop())
 if len(sys.argv)==3 and sys.argv[1]=='is-desktop':
  d=state()['desktop'];raise SystemExit(0 if d and d['address']==sys.argv[2] else 1)
 print(json.dumps(state()))
