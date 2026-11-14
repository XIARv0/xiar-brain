# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Compositor-chrome child only. Bounded heartbeat/stop; no cookies, keys or resume/widen authority."""
import http.client,json,socket,sys,tomllib
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
import paths as xiar_paths
cfg=tomllib.loads(xiar_paths.policy_path().read_text());action=sys.argv[1] if len(sys.argv)>1 else ''
if action not in ('heartbeat','stop'):raise SystemExit(2)
body=json.dumps(dict(action=action,session_id=sys.argv[2] if len(sys.argv)>2 else '')).encode()
class Unix(http.client.HTTPConnection):
 def connect(self):self.sock=socket.socket(socket.AF_UNIX);self.sock.settimeout(cfg['control']['state_probe_timeout_s']);self.sock.connect(cfg['control']['broker_socket'])
c=Unix('localhost',timeout=cfg['control']['state_probe_timeout_s']);c.request('POST','/internal/drive-chrome',body,{'Content-Type':'application/json'});response=c.getresponse();response.read(4096);c.close()
if response.status!=200:raise SystemExit(1)
