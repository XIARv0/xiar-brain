#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Dev-only autonomy broker for local memory writes. Powered by XIAR V0 - https://xiarv0.com

Publishes autonomy-state.json and answers POST /internal/emergency on the Unix socket so a
production (non test-hooks) xiar-memory-api can admit inserts. This is not the privileged
production broker in server.py: it does not enforce tool policy, drive the desktop, or require root.

  export XIAR_ROOT=/path/to/repo
  export XIAR_AUTONOMY_DEV=1
  export XIAR_AUTONOMY_CONFIG=$XIAR_ROOT/config/autonomy.dev.toml
  python integrations/agent-control/dev_broker.py            # running (writes allowed)
  python integrations/agent-control/dev_broker.py --stop     # emergency stop
  python integrations/agent-control/dev_broker.py --resume
"""
from __future__ import annotations

import argparse
import http.server
import json
import os
import signal
import socket
import socketserver
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import paths


class _State:
    def __init__(self):
        self.lock = threading.Lock()
        self.stopped = False
        self.generation = 0
        self.cutoff = 0.0

    def snapshot(self) -> dict:
        with self.lock:
            return dict(stopped=self.stopped, generation=self.generation, cutoff=self.cutoff, source='dev_broker')

    def set_stopped(self, stopped: bool) -> dict:
        with self.lock:
            self.generation += 1
            self.stopped = bool(stopped)
            if self.stopped:
                self.cutoff = time.time()
        value = self.snapshot()
        publish(value)
        return value


def socket_path(cfg: dict) -> Path:
    if raw := os.environ.get('XIAR_AUTONOMY_SOCKET', '').strip():
        return paths.resolve_path(raw)
    raw = (cfg.get('control') or {}).get('broker_socket') or 'data/dev/broker.sock'
    return paths.resolve_path(raw)


def publish(value: dict) -> None:
    path = paths.state_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + '.new')
    data = json.dumps(value, allow_nan=False).encode()
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o644)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    os.chmod(tmp, 0o644)
    os.replace(tmp, path)


class Control(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True
    allow_reuse_address = True


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def reply(self, status: int, data: dict):
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        try:
            length = int(self.headers.get('Content-Length', '0') or 0)
            if length:
                self.rfile.read(min(length, 65536))
            if self.path != '/internal/emergency':
                return self.reply(404, {'error': 'Unknown dev-broker route'})
            return self.reply(200, STATE.snapshot())
        except Exception:
            return self.reply(503, {'error': 'Dev broker unavailable'})

    def do_GET(self):
        return self.do_POST() if self.path == '/internal/emergency' else self.reply(404, {'error': 'Unknown dev-broker route'})


STATE = _State()


def load_cfg() -> dict:
    p = paths.policy_path()
    if not p.is_file():
        raise SystemExit(f'Autonomy policy not found: {p}. Set XIAR_AUTONOMY_CONFIG to config/autonomy.dev.toml')
    import tomllib
    return tomllib.loads(p.read_text(encoding='utf-8'))


def bind(sock: Path) -> Control:
    sock.parent.mkdir(parents=True, exist_ok=True)
    if sock.exists():
        sock.unlink()
    server = Control(str(sock), Handler)
    os.chmod(sock, 0o660)
    return server


def main() -> int:
    parser = argparse.ArgumentParser(description='Dev-only XIAR autonomy broker (local memory writes).')
    parser.add_argument('--stop', action='store_true', help='start in emergency-stop (reads only)')
    parser.add_argument('--resume', action='store_true', help='start running (memory writes allowed); default')
    args = parser.parse_args()
    if not paths.dev_mode():
        raise SystemExit('Refusing to start: set XIAR_AUTONOMY_DEV=1 and XIAR_AUTONOMY_CONFIG to a file other than /etc/xiar/autonomy.toml')
    cfg = load_cfg()
    sock = socket_path(cfg)
    STATE.set_stopped(bool(args.stop) and not args.resume)
    server = bind(sock)
    print(f'dev broker listening on {sock}  state={paths.state_path()}  stopped={STATE.snapshot()["stopped"]}', flush=True)

    def shutdown(*_):
        server.shutdown()

    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)
    try:
        server.serve_forever()
    finally:
        try:
            server.server_close()
        finally:
            if sock.exists():
                sock.unlink()
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
