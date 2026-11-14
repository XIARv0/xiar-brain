# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Watches the private Agents' services (a thread of the root broker) and keeps a record of every start and shutdown, with the reason.

What it decides
- A shutdown that someone asked for is "stopped" and is NEVER restarted by this monitor: it was recorded as an intent by the broker (a stop or re-render the
  administrator or an approved Agent action asked for), or the service manager's journal shows a stop job for that run (someone ran systemctl stop).
- A shutdown nobody asked for is "crashed". systemd (Restart=on-failure) restarts it at once; when systemd gives up (start limit) this monitor resets the
  failure and starts it again, waiting longer each time, and records "gave-up" when it stops trying (until an administrator starts the Agent again).
- Hermes exits with status 1 on any stop that is not its own planned stop, so a stop looks like a failure to systemd; the intent / stop job tells them apart.
Events go to a root-only log and to Settings > Agents > Maintenance. No token or Agent content is ever recorded."""
import collections,json,os,pathlib,subprocess,threading,time
import runtime

EVENTS=runtime.CONTROL/'agent-events.jsonl'
INTENTS=runtime.CONTROL/'intents'
POLL_S=3.0
INTENT_TTL_S=180
RETRY_DELAYS_S=(30,120,600)     # waits before the 1st, 2nd and 3rd restart attempt of a crashed Agent that systemd gave up on
WINDOW_S=3600                   # attempts are counted per hour
MEMORY=collections.deque(maxlen=200)
LOCK=threading.RLock()
PROPS='ActiveState,SubState,Result,NRestarts,InvocationID,ExecMainStatus,ExecMainCode'

def intent(profile,by,reason):
    """Recorded by the broker just before it stops or restarts an Agent on purpose, so the shutdown is read as requested."""
    runtime.private(INTENTS)
    runtime.atomic_root(INTENTS/(runtime.slug(profile)+'.json'),json.dumps(dict(by=by,reason=reason,at=time.time())).encode())

def _take_intent(profile):
    path=INTENTS/(profile+'.json')
    try:
        value=json.loads(path.read_text());path.unlink()
    except (OSError,ValueError):return None
    return value if time.time()-float(value.get('at',0))<=INTENT_TTL_S else None

def record(**event):
    event['at']=time.time()
    with LOCK:
        MEMORY.append(event)
        runtime.private(runtime.CONTROL)
        fd=os.open(EVENTS,os.O_WRONLY|os.O_CREAT|os.O_APPEND|os.O_NOFOLLOW,0o600)
        with os.fdopen(fd,'a') as out:out.write(json.dumps(event)+'\n')
    return event

def events(limit=50):
    try:lines=EVENTS.read_text().splitlines()[-max(1,min(int(limit),500)):]
    except OSError:return []
    out=[]
    for line in lines:
        try:out.append(json.loads(line))
        except ValueError:pass
    return out

def _show(unit):
    q=subprocess.run(['systemctl','show',unit,'--property='+PROPS],capture_output=True,text=True,timeout=20)
    return dict(line.split('=',1) for line in q.stdout.splitlines() if '=' in line)

def _stop_job_in_journal(unit):
    """True when the service manager logged a stop job for this run (somebody ran systemctl stop / the broker stopped it)."""
    q=subprocess.run(['journalctl','-u',unit,'--since','-10min','-o','cat','--no-pager'],capture_output=True,text=True,timeout=30)
    last_started=-1;stopping=-1
    for index,line in enumerate(q.stdout.splitlines()):
        if line.startswith('Started '):last_started=index
        if line.startswith('Stopping '):stopping=index
    return stopping!=-1 and stopping>last_started

class Monitor(threading.Thread):
    daemon=True
    def __init__(self):
        super().__init__(name='agent-monitor')
        self.seen={};self.kind={};self.attempts=collections.defaultdict(list);self.failed_since={};self.gave_up=set()
    def scopes(self):
        state=runtime.state();rows=[]
        for row in state['scopes'].values():
            cfg=state['agents'].get(row['agent'],{})
            if cfg.get('linux_user'):rows.append((row['profile'],row['agent'],bool(row.get('active'))))
        return rows
    def run(self):
        while True:
            try:self.once()
            except Exception:pass
            time.sleep(POLL_S)
    def once(self):
        now=time.time()
        for profile,agent,marked_active in self.scopes():
            unit=runtime.profile_unit(profile);s=_show(unit);prev=self.seen.get(profile)
            detail=dict(result=s.get('Result'),sub=s.get('SubState'),restarts=s.get('NRestarts'),exit_status=s.get('ExecMainStatus'),exit_code=s.get('ExecMainCode'))
            if prev and prev['ActiveState']=='active' and (s['ActiveState']!='active' or s.get('InvocationID')!=prev.get('InvocationID')):
                given=_take_intent(profile)
                if given:self.kind[profile]='stopped';record(kind='stopped',agent=agent,profile=profile,by=given['by'],reason=given['reason'],**detail)
                elif _stop_job_in_journal(unit):self.kind[profile]='stopped';record(kind='stopped',agent=agent,profile=profile,by='service manager stop (an administrator or a tool ran systemctl stop)',reason='requested stop',**detail)
                else:
                    self.kind[profile]='crashed';self.failed_since[profile]=now;self.gave_up.discard(profile)
                    record(kind='crashed',agent=agent,profile=profile,by='nobody asked for it',reason='the service ended on its own; systemd restarts it unless it gives up',**detail)
            if s['ActiveState']=='active' and (not prev or prev['ActiveState']!='active' or s.get('InvocationID')!=prev.get('InvocationID')):
                self.kind[profile]='running';self.failed_since.pop(profile,None);self.gave_up.discard(profile)
                if prev:record(kind='started',agent=agent,profile=profile,by='systemd or the broker',reason='running again',**detail)
            # systemd gave up on a crashed Agent that should be running: try again, patiently, a few times
            if s['ActiveState']=='failed' and self.kind.get(profile)=='crashed' and marked_active and profile not in self.gave_up:
                started=self.failed_since.setdefault(profile,now);recent=[t for t in self.attempts[profile] if now-t<WINDOW_S];self.attempts[profile]=recent
                if len(recent)>=len(RETRY_DELAYS_S):
                    self.gave_up.add(profile);record(kind='gave-up',agent=agent,profile=profile,by='monitor',reason='crashed repeatedly; not restarting again until an administrator starts it',**detail)
                elif now-started>=RETRY_DELAYS_S[len(recent)]:
                    self.attempts[profile].append(now)
                    subprocess.run(['systemctl','reset-failed',unit],capture_output=True,timeout=30);subprocess.run(['systemctl','start',unit],capture_output=True,timeout=100)
                    record(kind='restart-attempt',agent=agent,profile=profile,by='monitor',reason='attempt %d after a crash systemd gave up on'%len(self.attempts[profile]),**detail)
            self.seen[profile]=s

def states():
    out=[]
    state=runtime.state()
    for row in state['scopes'].values():
        s=_show(runtime.profile_unit(row['profile']))
        out.append(dict(agent=row['agent'],profile=row['profile'],active=s.get('ActiveState'),sub=s.get('SubState'),result=s.get('Result'),restarts=s.get('NRestarts')))
    return out
