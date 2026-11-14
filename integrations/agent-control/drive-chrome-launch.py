# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Fixed root-installed chrome unit launcher. No inherited QML/loader configuration or user fallback."""
from pathlib import Path
import json,os,pwd,re,stat,subprocess,sys,tomllib
sys.path.insert(0, str(Path(__file__).resolve().parent))
import paths as xiar_paths
cfg=tomllib.loads(xiar_paths.policy_path().read_text())['drive']
runtime=xiar_paths.desktop_runtime_dir(cfg);uid=os.getuid();info=runtime.lstat()
want=xiar_paths.desktop_uid(cfg)
if uid!=want or not stat.S_ISDIR(info.st_mode) or info.st_uid!=uid or info.st_mode&0o077:raise SystemExit('Unsafe desktop runtime')
env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8','LC_ALL':'C.UTF-8','XDG_RUNTIME_DIR':str(runtime)}
q=subprocess.run([cfg['hyprctl'],'-j','instances'],env=env,capture_output=True,timeout=5,check=True)
if len(q.stdout)>1048576:raise SystemExit('Compositor metadata too large')
instances=json.loads(q.stdout)
if not isinstance(instances,list) or len(instances)!=1:raise SystemExit('A unique real compositor is required')
signature=instances[0].get('instance','');display=instances[0].get('wl_socket','')
if not re.fullmatch('[a-zA-Z0-9_.-]+',signature) or not re.fullmatch('wayland-[a-zA-Z0-9_-]+',display):raise SystemExit('Invalid compositor metadata')
s=(runtime/display).lstat()
if not stat.S_ISSOCK(s.st_mode) or s.st_uid!=uid:raise SystemExit('Unsafe Wayland socket')
for name in ('indicator_parent_executable','chrome_qml'):
 p=Path(cfg[name]).resolve(strict=True);s=p.stat()
 if not stat.S_ISREG(s.st_mode) or s.st_uid!=0 or s.st_mode&0o022:raise SystemExit('Root-installed chrome required')
project=cfg.get('project_root') or str(xiar_paths.project_root())
env.update(XIAR_ROOT=project,HOME=pwd.getpwuid(uid).pw_dir,USER=pwd.getpwuid(uid).pw_name,WAYLAND_DISPLAY=display,HYPRLAND_INSTANCE_SIGNATURE=signature,QT_QPA_PLATFORM='wayland',QSG_RHI_BACKEND='software',QT_QUICK_BACKEND='software',LIBGL_ALWAYS_SOFTWARE='1',QML_DISABLE_DISK_CACHE='1',XDG_CACHE_HOME='/var/cache/xiar-desktop-chrome',XDG_CONFIG_HOME='/etc/xiar/desktop')
os.execve(cfg['indicator_parent_executable'],[cfg['indicator_parent_executable'],'-p',str(Path(cfg['chrome_qml']).parent)],env)
