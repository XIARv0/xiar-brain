#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""XIAR loops: Reflex (fast, no LLM), Circadian (every 1-4 h), Dream (once a day). Stdlib only. See OSLayer.md and config/loops.toml.

  xiar_loops.py reflex      daemon: sample, detect a CHANGE, back off when bored, log a trigger, wake at most `max_wakes_per_hour`
  xiar_loops.py circadian   one run (systemd timer): digest of what changed since the last run, skipped when nothing did
  xiar_loops.py dream       one run (systemd timer): prune/de-duplicate the event log, record the last dream
  xiar_loops.py status      print the status file

Guardrails (mandatory, all deterministic): state-change activation, boredom backoff, hard caps with a logged "cognitive panic", and the emergency stop always wins:
a stopped or unreadable emergency state means no wake is ever requested. Window TITLES, keystrokes and pixels are never read or stored."""
import hashlib, json, os, pathlib, shutil, subprocess, sys, time, tomllib

CONFIG = pathlib.Path(os.environ.get('XIAR_LOOPS_CONFIG', pathlib.Path(__file__).resolve().parents[2] / 'config/loops.toml'))


def load(path=CONFIG):
    cfg = tomllib.loads(pathlib.Path(path).read_text(encoding='utf-8'))
    for k, v in cfg['paths'].items():
        if k != 'autonomy_state': cfg['paths'][k] = str(pathlib.Path(v).expanduser())
    return cfg


def stopped(cfg):
    """Unreadable, unsafe or stopped = stopped (fail closed), exactly like the other safety consumers."""
    p = pathlib.Path(cfg['paths']['autonomy_state'])
    try:
        info = p.lstat()
        # inside a sandboxed user service real root shows as the kernel's overflow user, so 0 and that uid both count as root
        try: overflow = int(open('/proc/sys/kernel/overflowuid').read())
        except (OSError, ValueError): overflow = 65534
        if not (info.st_mode & 0o170000 == 0o100000) or info.st_uid not in (0, overflow) or info.st_mode & 0o022: return True
        return bool(json.loads(p.read_text())['stopped'])
    except (OSError, ValueError, KeyError): return True


# ---- sensors (each returns plain numbers; none reads content)
def sample(cfg):
    out = {}
    try:
        load1 = float(open('/proc/loadavg').read().split()[0]); out['load_ratio'] = load1 / (os.cpu_count() or 1)
    except OSError: out['load_ratio'] = 0.0
    try:
        mem = {l.split(':')[0]: int(l.split()[1]) for l in open('/proc/meminfo')}
        out['mem_ratio'] = 1 - mem['MemAvailable'] / mem['MemTotal']
    except (OSError, KeyError, ValueError): out['mem_ratio'] = 0.0
    try:
        du = shutil.disk_usage('/'); out['disk_ratio'] = du.used / du.total; out['disk_used_mb'] = du.used // 1048576
    except OSError: out['disk_ratio'] = 0.0; out['disk_used_mb'] = 0
    temps = []
    for z in pathlib.Path('/sys/class/thermal').glob('thermal_zone*/temp'):
        try: temps.append(int(z.read_text()) / 1000)
        except (OSError, ValueError): pass
    out['temp_c'] = max(temps) if temps else 0.0
    out['window_class'] = active_window_class()
    return out


def active_window_class():
    """Only the window CLASS (an application name), never its title."""
    try:
        runtime = pathlib.Path(f'/run/user/{os.getuid()}'); found = sorted((runtime / 'hypr').glob('*/')) if (runtime / 'hypr').is_dir() else []
        if len(found) != 1: return ''   # no compositor, or not exactly one: say nothing
        q = subprocess.run(['/usr/bin/hyprctl', '-j', 'activewindow'], capture_output=True, timeout=3, env={'PATH': '/usr/bin:/bin', 'XDG_RUNTIME_DIR': str(runtime), 'HYPRLAND_INSTANCE_SIGNATURE': found[0].name})
        return str(json.loads(q.stdout).get('class', ''))[:80] if q.returncode == 0 else ''
    except (OSError, ValueError, subprocess.SubprocessError): return ''


# ---- the engine: pure, so it is testable without a machine
class Engine:
    def __init__(self, cfg):
        self.cfg = cfg; self.last = None; self.active = set(); self.interval = float(cfg['reflex']['interval_s'])
        self.focus = []; self.wakes = []; self.panic_hour = None

    def conditions(self, s, now):
        r = self.cfg['reflex']; c = set()
        if s['load_ratio'] >= r['cpu_load_ratio']: c.add('cpu')
        if s['mem_ratio'] >= r['mem_used_ratio']: c.add('memory')
        if s['disk_ratio'] >= r['disk_used_ratio']: c.add('disk-full')
        if self.last and s['disk_used_mb'] - self.last['disk_used_mb'] >= r['disk_delta_mb']: c.add('disk-growth')
        if s['temp_c'] >= r['temp_c']: c.add('thermal')
        if s['window_class']:
            if not self.focus or self.focus[-1][1] != s['window_class']: self.focus.append((now, s['window_class']))
            self.focus = [f for f in self.focus if now - f[0] <= r['focus_window_s']]
            if len(self.focus) >= r['focus_switches']: c.add('focus-churn')
        return c

    def step(self, s, now):
        """-> (newly active conditions or None, next interval). State-change activation: only a condition that was not active before counts."""
        c = self.conditions(s, now); r = self.cfg['reflex']
        new = c - self.active; self.active = c; self.last = s
        if new and len(new) >= r['state_change_min']:
            self.interval = float(r['interval_s']); return sorted(new), self.interval
        self.interval = float(r['interval_s']) if c else min(self.interval * r['backoff_factor'], float(r['interval_max_s']))
        return None, max(float(r['interval_min_s']), self.interval)

    def may_wake(self, now, is_stopped):
        w = self.cfg['wake']
        if is_stopped: return False, 'stopped'
        if w['mode'] == 'log': return False, 'log-only'
        hour = int(now // 3600)
        if self.panic_hour == hour: return False, 'panic'
        self.wakes = [t for t in self.wakes if now - t < 3600]
        if self.wakes and now - self.wakes[-1] < w['cooldown_s']: return False, 'cooldown'
        if len(self.wakes) >= w['max_wakes_per_hour']: self.panic_hour = hour; return False, 'panic'
        self.wakes.append(now); return True, 'ok'


def log_event(cfg, kind, **fields):
    p = pathlib.Path(cfg['paths']['events_log']); p.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with open(p, 'a', encoding='utf-8') as f: f.write(json.dumps(dict(t=time.time(), kind=kind, **fields), sort_keys=True) + '\n')


def write_atomic(path, text):
    p = pathlib.Path(path); p.parent.mkdir(parents=True, exist_ok=True, mode=0o700); tmp = p.with_suffix(p.suffix + '.tmp'); tmp.write_text(text, encoding='utf-8'); os.replace(tmp, p)


def identity_block(cfg, s):
    dream = '?'
    try: dream = time.strftime('%Y-%m-%d %H:%M', time.localtime(json.loads(pathlib.Path(cfg['paths']['state_dir'], 'dream.json').read_text())['at']))
    except (OSError, ValueError, KeyError): pass
    return f"Running on {os.cpu_count()} CPUs, memory {s['mem_ratio']*100:.0f}% used, disk {s['disk_ratio']*100:.0f}% used, load {s['load_ratio']:.2f}, last dream {dream}.\n"


def reflex(cfg, once=False):
    e = Engine(cfg)
    while True:
        now = time.time(); s = sample(cfg); is_stopped = stopped(cfg); trig, wait = e.step(s, now)
        status = dict(at=now, stopped=is_stopped, interval_s=wait, active=sorted(e.active), sample={k: s[k] for k in ('load_ratio', 'mem_ratio', 'disk_ratio', 'temp_c')})
        if trig:
            ok, why = e.may_wake(now, is_stopped)
            log_event(cfg, 'trigger', conditions=trig, wake=ok, reason=why, sample=status['sample'])
            if why == 'panic': log_event(cfg, 'cognitive-panic', wakes_last_hour=len(e.wakes))
            if ok: write_atomic(pathlib.Path(cfg['paths']['state_dir'], 'wake-request.json'), json.dumps(dict(at=now, conditions=trig, max_seconds=cfg['wake']['max_wake_seconds'])))
        write_atomic(cfg['paths']['status_file'], json.dumps(status)); write_atomic(cfg['paths']['identity_block'], identity_block(cfg, s))
        if once: return status
        time.sleep(wait)


def read_events(cfg, since=0.0):
    p = pathlib.Path(cfg['paths']['events_log'])
    try: rows = [json.loads(l) for l in p.read_text(encoding='utf-8').splitlines() if l.strip()]
    except (OSError, ValueError): return []
    return [r for r in rows if r.get('t', 0) > since]


def circadian(cfg):
    d = pathlib.Path(cfg['paths']['state_dir']); sp = d / 'circadian.json'
    try: st = json.loads(sp.read_text())
    except (OSError, ValueError): st = dict(at=0.0, skip=0, owed=0)
    if st.get('owed', 0) > 0:
        st['owed'] -= 1; write_atomic(sp, json.dumps(st)); log_event(cfg, 'circadian-skipped', owed=st['owed']); return dict(skipped=True)
    rows = [r for r in read_events(cfg, st['at']) if r['kind'] == 'trigger']
    now = time.time()
    if not rows:
        st['skip'] = min(max(1, st.get('skip', 0) * 2), cfg['circadian']['max_skip']); st['owed'] = st['skip']; st['at'] = now
        write_atomic(sp, json.dumps(st)); log_event(cfg, 'circadian-quiet', next_skip=st['skip']); return dict(quiet=True, next_skip=st['skip'])
    counts = {}
    for r in rows:
        for c in r['conditions']: counts[c] = counts.get(c, 0) + 1
    digest = dict(at=now, since=st['at'], triggers=len(rows), conditions=counts)
    write_atomic(d / 'circadian-digest.json', json.dumps(digest)); write_atomic(sp, json.dumps(dict(at=now, skip=0, owed=0)))
    log_event(cfg, 'circadian-digest', **{k: v for k, v in digest.items() if k != 'at'})
    return digest


def dream(cfg):
    p = pathlib.Path(cfg['paths']['events_log']); now = time.time(); kept = []; seen = set(); before = 0
    try: lines = p.read_text(encoding='utf-8').splitlines()
    except OSError: lines = []
    cutoff = now - cfg['dream']['keep_days'] * 86400
    for l in lines:
        before += 1
        try: row = json.loads(l)
        except ValueError: continue
        key = hashlib.sha256(json.dumps({k: v for k, v in row.items() if k != 't'}, sort_keys=True).encode()).hexdigest()
        if row.get('t', 0) < cutoff or (row['kind'] == 'trigger' and key in seen): continue
        seen.add(key); kept.append(l)
    while kept and sum(len(x) + 1 for x in kept) > cfg['dream']['max_log_mb'] * 1048576: kept.pop(0)
    if lines: write_atomic(p, '\n'.join(kept) + ('\n' if kept else ''))
    out = dict(at=now, before=before, after=len(kept)); write_atomic(pathlib.Path(cfg['paths']['state_dir'], 'dream.json'), json.dumps(out))
    log_event(cfg, 'dream', before=before, after=len(kept)); return out


if __name__ == '__main__':
    mode = sys.argv[1] if len(sys.argv) > 1 else 'status'; cfg = load()
    if mode == 'reflex': reflex(cfg)
    elif mode == 'circadian': print(json.dumps(circadian(cfg)))
    elif mode == 'dream': print(json.dumps(dream(cfg)))
    else: print(pathlib.Path(cfg['paths']['status_file']).read_text())
