# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Owner-loopback OpenAI-compatible streaming provider adapter; never follows redirects."""
import http.server,json,os,pathlib,sys,tomllib,urllib.request,urllib.error
import importlib.util
spec=importlib.util.spec_from_file_location('providers',pathlib.Path(__file__).with_name('call.py'));p=importlib.util.module_from_spec(spec);spec.loader.exec_module(p)
ID=sys.argv[1];c=p.connections(tomllib.loads((p.ROOT/'config/llm_runtime.toml').read_text()))[ID];port=int(c['proxy_port'])
if not 1023<port<65536:raise SystemExit('invalid provider proxy port')
class H(http.server.BaseHTTPRequestHandler):
 def log_message(self,*_):pass
 def error(self,status):
  data=json.dumps({'error':{'message':'Configured provider unavailable or request refused'}}).encode();self.send_response(status);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
 def process(self,post):
  if self.headers.get('Host') not in ('127.0.0.1:'+str(port),'localhost:'+str(port)):return self.error(403)
  if self.headers.get('Origin'):return self.error(403) # server-to-server only
  if self.path not in ('/v1/models','/v1/chat/completions','/v1/embeddings'):return self.error(404)
  if post and self.headers.get_content_type()!='application/json':return self.error(415)
  started=False
  try:
   data=None
   if post:
    length=int(self.headers.get('Content-Length','0'))
    if not 0<length<=int(c.get('max_request_bytes',4194304)):return self.error(413)
    body=json.loads(self.rfile.read(length));model=c.get('served_name') or c.get('model')
    if body.get('model')!=model:return self.error(403)
    if self.path=='/v1/embeddings' and c.get('kind')!='embedding':return self.error(403)
    if self.path=='/v1/chat/completions' and c.get('kind')!='chat':return self.error(403)
    data=json.dumps(body).encode()
   headers={'Content-Type':'application/json'};ref=c.get('credential_env')
   if ref:
    value=os.environ.get(ref)
    if not value:return self.error(503)
    headers['Authorization']='Bearer '+value
   req=urllib.request.Request(p.api_url(c,self.path),data=data,headers=headers)
   with urllib.request.build_opener(p.NoRedirect).open(req,timeout=float(c.get('timeout_s',300))) as response:
    if self.path=='/v1/models':
     raw=response.read(int(c.get('max_response_bytes',4194304))+1);obj=json.loads(raw)
     for row in obj.get('data',[]):
      if row.get('id')==(c.get('served_name') or c.get('model')):
       if c.get('dim'):row.setdefault('embedding_dimension',c['dim'])
       if c.get('revision'):row.setdefault('fingerprint',c['revision'])
     out=json.dumps(obj).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(out)));self.end_headers();self.wfile.write(out)
    else:
     self.send_response(200);self.send_header('Content-Type','text/event-stream' if post and body.get('stream') else 'application/json');self.send_header('Connection','close');self.end_headers();started=True
     total=0;limit=int(c.get('max_response_bytes',4194304))
     while True:
      chunk=response.read1(65536)
      if not chunk:break
      total+=len(chunk)
      if total>limit:raise ValueError("provider stream exceeded bound")
      self.wfile.write(chunk);self.wfile.flush()
  except Exception:
   # Never return provider response bodies or runtime credentials to a client.
   try:
    if not started:self.error(502)
    else:self.close_connection=True
   except OSError:pass
 def do_GET(self):self.process(False)
 def do_POST(self):self.process(True)
http.server.ThreadingHTTPServer(('127.0.0.1',port),H).serve_forever()
