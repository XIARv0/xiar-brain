# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Owner desktop asks the peer-gated broker to start its fixed chrome service. No model/API credentials."""
import http.client,json,socket,time,tomllib
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
import paths as xiar_paths
cfg=tomllib.loads(xiar_paths.policy_path().read_text());deadline=time.monotonic()+float(cfg['drive']['chrome_start_timeout_s'])
class Unix(http.client.HTTPConnection):
 def connect(self):self.sock=socket.socket(socket.AF_UNIX);self.sock.settimeout(cfg['drive']['chrome_start_timeout_s']);self.sock.connect(cfg['control']['broker_socket'])
while True:
 c=Unix('localhost')
 try:
  c.request('POST','/internal/desktop-chrome',b'{}',{'Content-Type':'application/json'});r=c.getresponse();body=r.read(4096)
  if r.status!=200:raise SystemExit('Root desktop chrome start refused; no user-QML fallback')
  if not json.loads(body).get('started'):raise SystemExit('Desktop chrome did not start')
  break
 except (OSError,http.client.HTTPException):
  if time.monotonic()>=deadline:raise SystemExit('Desktop broker unavailable; no untrusted fallback')
  time.sleep(float(cfg['control']['poll_s']))
 finally:c.close()
