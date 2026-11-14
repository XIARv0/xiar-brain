# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Configured provider adapter: no shell, redirects, credential values in settings or logs."""
import signal
import json,os,pathlib,re,sys,tomllib,urllib.request,urllib.parse
ROOT=pathlib.Path(os.environ.get('XIAR_ROOT',pathlib.Path(__file__).resolve().parents[2]))
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*a,**k):return None

def connections(runtime):
    out={}
    for key,kind in [('chat','chat'),('embedding','embedding')]:
        c=dict(runtime.get(key,{}));c.setdefault('kind',kind);c.setdefault('type','local');out[c.get('served_name',key)]=c
    out.update(runtime.get('connections',{}));return out

def endpoint(c):
    base=c.get('base_url') or 'http://'+c.get('host','127.0.0.1')+':'+str(c.get('port',0))
    u=urllib.parse.urlsplit(base)
    if u.username or u.password or u.query or u.fragment or u.scheme not in ('http','https') or not u.hostname:raise ValueError('Invalid endpoint')
    if u.scheme=='http' and u.hostname not in ('localhost','127.0.0.1','::1'):raise ValueError('Remote providers require TLS')
    return base.rstrip('/')

def api_url(c,path):
    base=endpoint(c)
    if path.startswith('/v1/'):
        return base+path[3:] if urllib.parse.urlsplit(base).path.rstrip('/') else base+path
    return base.removesuffix('/v1')+path

def request(c,path,body=None):
    h={'Content-Type':'application/json'};ref=c.get('credential_env','')
    if ref:
        if not re.fullmatch('[A-Z][A-Z0-9_]{0,127}',ref):raise ValueError('Invalid credential reference')
        value=os.environ.get(ref)
        if not value:raise ValueError('Configured credential is unavailable')
        h['Authorization']='Bearer '+value
    url=api_url(c,path);data=None if body is None else json.dumps(body).encode()
    with urllib.request.build_opener(NoRedirect).open(urllib.request.Request(url,data=data,headers=h),timeout=float(c.get('timeout_s',300))) as response:
        raw=response.read(int(c.get('max_response_bytes',4194304))+1)
    if len(raw)>int(c.get('max_response_bytes',4194304)):raise ValueError('Provider response too large')
    return json.loads(raw)

def call(req):
    runtime=tomllib.loads((ROOT/'config/llm_runtime.toml').read_text());all=connections(runtime);c=all[req['connection']]
    if c.get('kind')!='chat':raise ValueError('Population needs a chat/instruct connection')
    panel=tomllib.loads((ROOT/'config/settings_panel.toml').read_text());opts=panel.get('memory',{}).get('job_opts',{}).get(req.get('job',''),{})
    dev=lambda d:'gpu' if d=='cuda' else d
    target=dev(opts.get('device','auto'));actual=dev(c.get('device','auto'))
    if c.get('type','local')=='local':
        if target!='auto' and target!=actual:raise ValueError('Use a served connection with the selected placement')
        if c.get('mistralrs_form')=='builtin':
            active=runtime['chat']
            if pathlib.Path(c['model_path']).expanduser()!=pathlib.Path(active['model_path']).expanduser() or actual!=dev(active.get('device','auto')):raise ValueError('Built-in engine: one model, one processor including active chat')
    def deadline(*_):raise TimeoutError('Provider wall deadline')
    signal.signal(signal.SIGALRM,deadline);signal.alarm(max(1,min(86400,int(c.get('timeout_s',300)))))
    model=c.get('served_name') or c.get('model') or req['connection']
    messages=req['messages'];maximum=int(req.get('max_tokens',2048));thinking=c.get('thinking','auto')
    if c.get('type')=='ollama':
        body={'model':model,'messages':messages,'stream':False,'options':{'num_predict':maximum,'num_ctx':int(c.get('context_length',8192))}}
        if thinking!='auto':body['think']=thinking=='on'
        response=request(c,'/api/chat',body);text=response['message']['content']
    else:
        body={'model':model,'messages':messages,'max_tokens':maximum,'temperature':0,'stream':False}
        if thinking!='auto':body.update(enable_thinking=thinking=='on',chat_template_kwargs={'enable_thinking':thinking=='on'})
        response=request(c,'/v1/chat/completions',body);text=response['choices'][0]['message']['content']
    if not isinstance(text,str):raise ValueError('Provider returned no structured content')
    # No regex repair or implicit tool execution: malformed extraction is retried sealed.
    result=json.loads(text)
    if not isinstance(result,dict):raise ValueError('Expected structured extraction object')
    return result
if __name__=='__main__':
    try:print(json.dumps(call(json.load(sys.stdin)),allow_nan=False))
    except Exception:
        print('{"error":"Configured provider or extraction failed; no credential or source content logged"}')
        sys.exit(2)
