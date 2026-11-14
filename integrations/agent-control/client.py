# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Unix transport only. Credentials are forwarded to the stock verifier, never logged."""
import http.client,json,pathlib,socket,tomllib
class UnixConnection(http.client.HTTPConnection):
    def __init__(self,path,timeout):super().__init__('localhost',timeout=timeout);self.path=path
    def connect(self):
        self.sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);self.sock.settimeout(self.timeout);self.sock.connect(self.path)
def connect(root):
    cfg=tomllib.loads((pathlib.Path(root)/'config/agent_control.toml').read_text(encoding='utf-8-sig'))['control']
    return UnixConnection(cfg['socket'],float(cfg['timeout_s']))
def call(root,command,args,headers):
    conn=connect(root)
    try:
        body=json.dumps(dict(command=command,args=args,authorization=headers.get('Authorization',''),cookie=headers.get('Cookie','')))
        conn.request('POST','/control',body,{'Content-Type':'application/json'})
        response=conn.getresponse();data=json.loads(response.read())
        if response.status!=200:raise ValueError(data.get('error','Authenticated Agent operation failed'))
        return data['result']
    finally:conn.close()
