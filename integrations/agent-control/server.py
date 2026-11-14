#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md

"""Authenticated privileged lifecycle + native prefix front. Installed root-owned.
No caller identity headers, shell commands, unit names, paths, users or ports trusted.
"""
import concurrent.futures,contextlib,hashlib,hmac,http.client,http.server,importlib.util,json,os,pathlib,pwd,socket,socketserver,struct,subprocess,sys,threading,tomllib,urllib.request,urllib.parse,urllib.error
import paths as xiar_paths
import runtime
from scheduler import Scheduler
SCHEDULER=None
AUTONOMY=None
ROOT=xiar_paths.project_root();OWNER=xiar_paths.owner();LOCK=threading.RLock()
def _load_cfg():
    p=xiar_paths.agent_control_config()
    return tomllib.loads(p.read_text())['control']
CFG=None  # loaded in main(); production needs agent-control.toml
AUTH_ROOT=xiar_paths.agent_control_auth()
AUTH=None
def auth_module():
    global AUTH
    if AUTH is None:
        path=pathlib.Path(__file__).with_name('settings_auth.py')
        if not path.is_file():
            raise RuntimeError('settings_auth.py is required for the privileged broker; use dev_broker.py for local memory writes')
        spec=importlib.util.spec_from_file_location('verified_stock_auth',path);AUTH=importlib.util.module_from_spec(spec);spec.loader.exec_module(AUTH)
    return AUTH
def fork(path,headers,data=None):
    base=auth_module().config(AUTH_ROOT)['base_url'];forward={k:headers[k] for k in ('Authorization','Cookie') if headers.get(k)}
    request=urllib.request.Request(base+path,data=json.dumps(data).encode() if data is not None else None,headers={'Content-Type':'application/json',**forward})
    try:
        with urllib.request.build_opener(urllib.request.ProxyHandler({}),auth_module().NoRedirect()).open(request,timeout=20) as response:return json.load(response)
    except Exception:raise ValueError('Stock model/group authorization or operation refused') from None
def child(command,args=None):
    # The Agent registry, every Agent's private home and its capability live in a ROOT-owned directory and are written only by this root broker, after the
    # administrator's sign-in. Nothing running as the owner's Linux user (the standing Xiar, Hermes tools) can read or edit them. Only the native Hermes
    # rescan needs the owner's own ~/.hermes, so only that runs as the owner.
    as_owner=command=='rescan'
    env={'PATH':'/usr/local/bin:/usr/bin:/bin','HOME':OWNER.pw_dir if as_owner else '/root','XIAR_ROOT':str(ROOT),'PYTHONPATH':'/opt/xiar/agent-control/python','PYTHONDONTWRITEBYTECODE':'1'}
    drop=dict(user=OWNER.pw_uid,group=OWNER.pw_gid,extra_groups=[]) if as_owner else {}
    proc=subprocess.run(['/usr/bin/python3','/opt/xiar/agent-control/profile-cli.py'],input=json.dumps({'command':command,'args':args or {}}),text=True,capture_output=True,timeout=120,env=env,**drop)
    if proc.returncode:raise ValueError('Private registry operation refused; existing data preserved')
    return json.loads(proc.stdout)['result']
def sync(agent):return runtime.synchronize(agent,int(CFG['runtime_port_first']),int(CFG['runtime_port_last']))
def activate(agent,profile):
    if AUTONOMY and AUTONOMY.current()['stopped']:raise ValueError('Emergency stop: start-when-used activation paused')
    child('activate',{'profile':profile});sync(agent)
    return runtime.ensure_running(agent,profile,float(CFG['startup_timeout_s']))
def lifecycle(command,args,user):
    if command=='agents_list':return child('list')
    if command=='agents_cold':
        import cold
        row=runtime.state()['scopes'].get(args.get('profile'))
        if not row:raise ValueError('Unknown private scope')
        if args.get('action')=='store':return cold.store(row['agent'],row['profile'])
        if args.get('action')=='restore':return dict(restored=cold.restore(row['agent'],row['profile']))
        return cold.status(row['profile'])
    if command=='agents_events':
        import agent_monitor
        return dict(events=agent_monitor.events(int(args.get('limit',30))),states=agent_monitor.states())
    if command=='agents_check':
        import agent_health
        return agent_health.check()
    if command=='agents_resync':
        # After a Hermes update: render every Agent's home, unit and grant again from the installed Hermes, restart the ones that were running, report.
        import agent_health
        agent=args.get('agent');targets=[runtime.slug(agent)] if agent else list(runtime.state()['agents'])
        for target in targets:
            if target not in runtime.state()['agents']:raise ValueError('Unknown Agent')
            running=[row['profile'] for row in runtime.state()['scopes'].values() if row['agent']==target and subprocess.run(['systemctl','is-active','--quiet',runtime.profile_unit(row['profile'])]).returncode==0]
            sync(target);child('ready',dict(agent=target,ready=True));sync(target)
            for profile in running:
                import agent_monitor;agent_monitor.intent(profile,'administrator (Settings)','restart to render again after a Hermes update')
                subprocess.run(['systemctl','restart',runtime.profile_unit(profile)],check=False,capture_output=True,timeout=100)
        return agent_health.check()
    if AUTONOMY and AUTONOMY.current()['stopped']:raise ValueError('Emergency stop: private lifecycle changes paused until owner resumes')
    if command=='agents_upsert':
        agent=args['id'];runtime.slug(agent)
        if agent in runtime.state()['agents']:child('freeze',{'agent':agent});runtime.stop(agent)
        result=child('upsert',args)
        for business in result['businesses']:child('scope',dict(agent=agent,verified_user=user['id'],business=business))
        sync(agent);child('ready',dict(agent=agent,ready=True));sync(agent)
        for row in runtime.state()['scopes'].values():
            if row['agent']==agent and row['enabled'] and row['active']:runtime.ensure_running(agent,row['profile'],float(CFG['startup_timeout_s']))
        return runtime.state()['agents'][agent]
    if command=='agent_scope':
        result=child('scope',dict(agent=args['agent'],verified_user=user['id'],business=args['business']));sync(args['agent'])
        if result['active']:runtime.ensure_running(args['agent'],result['profile'],float(CFG['startup_timeout_s']))
        return result
    if command=='agent_activate':
        row=runtime.state()['scopes'].get(args['profile'])
        if not row:raise ValueError('Unknown private scope')
        activate(row['agent'],row['profile']);return {'active':True}
    raise ValueError('Unknown lifecycle command')
def chat(args,headers,user):
    body=args['body'];model=body.get('model','')
    if not isinstance(model,str) or not model.startswith('agent/'):raise ValueError('Only Agent models use Rust/Hermes routing')
    agent=runtime.slug(model.split('/',1)[1])
    if AUTONOMY and AUTONOMY.current()['stopped'] and agent!='xiar':raise ValueError('Emergency stop: only standing Xiar read-only conversation is available')
    card=fork('/api/v1/models/model?id='+urllib.parse.quote(model,safe=''),headers)
    if not card or card.get('id')!=model or not card.get('is_active'):raise ValueError('Agent model not granted')
    cfg=runtime.state()['agents'].get(agent)
    if not cfg or not cfg['enabled']:raise ValueError('Agent disabled')
    allowed=card.get('meta',{}).get('xiar',{}).get('businesses',cfg['businesses'])
    business=body.pop('xiar_business',None)
    if business is None and len(allowed)==1:business=allowed[0]
    if business not in allowed or business not in cfg['businesses']:raise ValueError('Choose one explicitly granted business')
    with LOCK:
        scope=child('scope',dict(agent=agent,verified_user=user['id'],business=business))
        if not cfg.get('runtime_ready'):sync(agent);child('ready',dict(agent=agent,ready=True))
        port=activate(agent,scope['profile'])
        key=runtime.api_key(agent,scope['profile'])
    # Never forward auth cookies, caller identities, model/context metadata overrides.
    allowed_keys={'messages','stream','temperature','top_p','max_tokens','max_completion_tokens','stop','presence_penalty','frequency_penalty','seed','response_format','tools','tool_choice','parallel_tool_calls'}
    payload={k:v for k,v in body.items() if k in allowed_keys};payload.update(model=CFG['native_model'],user=user['id'],metadata={'xiar_agent':agent,'xiar_user':user['id'],'xiar_business':business})
    # Each scope's Hermes is single-profile on its own port with HERMES_HOME=/var/lib/xiar-agent in its sandbox, so it rejects a
    # /p/<profile>/ prefix ("Unknown or unconfigured profile", Hermes fails closed). The port already selects the scope (t154u fix).
    return port,'/v1/chat/completions',payload,{'Authorization':'Bearer '+key}
class Control(socketserver.ThreadingMixIn,socketserver.UnixStreamServer):daemon_threads=True
class TCP(socketserver.ThreadingMixIn,http.server.HTTPServer):daemon_threads=True;allow_reuse_address=True
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self,*a):pass
    def reply(self,status,data):
        encoded=json.dumps(data).encode();self.send_response(status);self.send_header('Content-Type','application/json');self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(encoded)));self.end_headers();self.wfile.write(encoded)
    def relay(self,port,path,body=None,headers=None):
        conn=http.client.HTTPConnection('127.0.0.1',port,timeout=float(CFG['timeout_s']))
        try:
            conn.request(self.command,path,json.dumps(body).encode() if isinstance(body,dict) else body,{'Content-Type':self.headers.get('Content-Type','application/json') if not isinstance(body,dict) else 'application/json',**(headers or {})})
            response=conn.getresponse();self.send_response(response.status);self.send_header('Content-Type',response.getheader('Content-Type','application/json'));self.send_header('Cache-Control','no-store');self.send_header('Connection','close');self.end_headers()
            while True:
                block=response.read1(16384)
                if not block:break
                self.wfile.write(block);self.wfile.flush()
        finally:conn.close();self.close_connection=True
    def read_body(self):
        size=int(self.headers.get('Content-Length','0'))
        if not 0<size<=int(CFG['max_body_bytes']):raise ValueError('Invalid request size')
        return self.rfile.read(size)
    def do_POST(self):
        if not isinstance(self.server,Control):return self.native()
        try:
            peer=struct.unpack('3i',self.connection.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12))
            if peer[1] not in (0,OWNER.pw_uid):return self.reply(403,{'error':'Caller UID refused'})
            if self.path=='/internal/desktop-chrome':
                self.read_body();return self.reply(200,AUTONOMY.drive.start_chrome(peer))
            if self.path=='/internal/drive-chrome':return self.reply(200,AUTONOMY.drive.chrome(peer,json.loads(self.read_body())))
            if self.path=='/internal/emergency':
                self.read_body()
                return self.reply(200,AUTONOMY.current())
            if self.path=='/internal/model-transport':
                from scheduler import private_token,owner_path
                cfg=tomllib.loads((ROOT/'config/model_router.toml').read_text())['model_router'];expected=private_token(owner_path(cfg['execution_token_file']),OWNER.pw_uid)
                if not hmac.compare_digest(self.headers.get('Authorization','').encode(),('Bearer '+expected).encode()):return self.reply(403,{'error':'Transport authority refused'})
                return self.reply(200,SCHEDULER.upstream.call(json.loads(self.read_body()),peer[0]))
            if self.path in ('/internal/scheduler','/internal/autonomy'):
                request=json.loads(self.read_body());profile=str(request.get('profile',''));authorization=str(request.get('authorization',''));body=request.get('body',{})
                if not isinstance(body,dict):raise ValueError('Scheduler object required')
                who=SCHEDULER.job_principal(profile[2:],authorization) if profile.startswith('j:') else SCHEDULER.principal(profile,authorization)
                if self.path=='/internal/autonomy':return self.reply(200,AUTONOMY.request(body,who))
                if body.get('action')=='verify':return self.reply(200,who)
                return self.reply(200,SCHEDULER.request(body,who))
            if self.path=='/internal/model-router/verify':
                request=json.loads(self.read_body());profile=request.get('profile','');row=runtime.state()['scopes'].get(profile)
                if not row or not row.get('enabled'):return self.reply(403,{'error':'Private routing scope disabled'})
                home=runtime.Profiles(ROOT).validate(row);path=home/'model-router-token'
                if path.is_symlink() or not path.is_file() or path.stat().st_mode&0o077:return self.reply(403,{'error':'Private routing credential unavailable'})
                if not hmac.compare_digest(str(request.get('authorization','')).encode(),('Bearer '+path.read_text().strip()).encode()):return self.reply(401,{'error':'Private routing capability refused'})
                return self.reply(200,{'profile':profile,'agent':row['agent'],'user':row['user'],'business':row['business']})
            if self.path!='/control':return self.reply(404,{'error':'Unknown control route'})
            request=json.loads(self.read_body());command=request['command'];args=request.get('args',{})
            headers={'Authorization':request.get('authorization',''),'Cookie':request.get('cookie','')}
            user=auth_module().verified(AUTH_ROOT,headers)
            if command=='chat':
                port,path,body,forward=chat(args,headers,user);return self.relay(port,path,body,forward)
            if user['role']!='admin':return self.reply(403,{'error':'Administrator role required'})
            if command.startswith('autonomy_'):return self.reply(200,{'result':AUTONOMY.admin(command,args,user)})
            with LOCK:result=lifecycle(command,args,user)
            return self.reply(200,{'result':result})
        except auth_module().Failure as error:return self.reply(error.status,{'error':str(error)})
        except (ValueError,KeyError,TypeError):return self.reply(400,{'error':'Agent operation refused; verify model grant, business, lifecycle configuration and runtime readiness'})
        except Exception:return self.reply(503,{'error':'Agent control unavailable; no owner/profile fallback'})
    def do_GET(self):return self.native()
    def do_DELETE(self):return self.native()
    def do_PUT(self):return self.native()
    def do_PATCH(self):return self.native()
    def do_OPTIONS(self):return self.native()
    def do_HEAD(self):return self.native()
    def native(self):
        try:
            path=urllib.parse.urlsplit(self.path).path;parts=path.split('/');port=int(CFG['owner_port'])
            if len(parts)>2 and parts[1]=='p':
                profile=parts[2];row=runtime.state()['scopes'].get(profile)
                if row:
                    if not row.get('enabled'):return self.reply(404,{'error':'Private profile unavailable'})
                    value=self.headers.get('Authorization','');key=runtime.api_key(row['agent'],profile)
                    if not hmac.compare_digest(value,'Bearer '+key):return self.reply(401,{'error':'Invalid private profile capability'})
                    with LOCK:port=activate(row['agent'],profile)
                    # same reason as chat(): the scope's own port, without the /p/<profile> prefix its Hermes would refuse
                    target=self.path[len('/p/'+profile):] or '/'
                    if not target.startswith(('/','?')):return self.reply(404,{'error':'Unknown private profile'})
                    if target.startswith('?'):target='/'+target
                    payload=self.read_body() if self.command in ('POST','PUT','PATCH') else None
                    return self.relay(port,target,payload,{'Authorization':self.headers.get('Authorization','')})
                elif profile.startswith('a-'):return self.reply(404,{'error':'Unknown private profile'})
            payload=self.read_body() if self.command in ('POST','PUT','PATCH') else None
            return self.relay(port,self.path,payload,{'Authorization':self.headers.get('Authorization','')})
        except Exception:return self.reply(503,{'error':'Native runtime unavailable; no profile fallback'})
def cold_sweeper():
    # Every few minutes: an Agent that starts only when used and has been stopped and untouched for a while goes to sealed cold storage (cold.py).
    import cold,time
    while True:
        time.sleep(600)
        idle=int(CFG.get('cold_after_s',21600))
        if idle<=0:continue
        try:
            with LOCK:cold.sweep(idle)
        except Exception:pass
def main():
    global SCHEDULER,AUTONOMY,CFG
    CFG=_load_cfg()
    from autonomy import Autonomy
    SCHEDULER=Scheduler();AUTONOMY=Autonomy(SCHEDULER);SCHEDULER.start()
    threading.Thread(target=cold_sweeper,daemon=True).start()
    import agent_monitor;agent_monitor.Monitor().start()
    path=pathlib.Path(CFG['socket']);path.parent.mkdir(parents=True,exist_ok=True);os.chown(path.parent,0,OWNER.pw_gid);os.chmod(path.parent,0o750)
    if path.exists():
        if not path.is_socket() or path.stat().st_uid!=0:raise SystemExit('Refusing unsafe control socket')
        path.unlink()
    control=Control(str(path),Handler);os.chown(path,0,OWNER.pw_gid);os.chmod(path,0o660)
    # Loopback native facade preserves the old port and exact native /p API paths.
    if CFG['facade_host']!='127.0.0.1' or int(CFG['facade_port'])==int(CFG['owner_port']):raise ValueError('Facade/owner require distinct loopback listeners')
    front=TCP((CFG['facade_host'],int(CFG['facade_port'])),Handler)
    threading.Thread(target=front.serve_forever,daemon=True).start();control.serve_forever()
if __name__=='__main__':main()
