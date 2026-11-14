# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Xiar's Paperclip tools as native Hermes tools (t145u, t146u, t150u).

Replaces the Hermes MCP link (which needed an MCP library Hermes could not install). This module talks to
Paperclip's OFFICIAL tool server (@paperclipai/mcp-server, kept at Paperclip's own version by
/opt/xiar/paperclip/paperclip-mcp.py) over its stdio protocol with only the standard library, and registers
each of its tools in Xiar's Hermes through the existing XIAR plugin. Only Xiar gets them; Agents never do.

XIAR's control (config/paperclip_tools.toml): a deny list, a per-call time limit, and a call log
(~/.local/state/xiar/paperclip-tools.jsonl, 0600: time, tool, argument NAMES, ok, duration; never values or keys).

  python -m xiar_hermes.paperclip_tools --refresh   rebuild the tool list after a Paperclip update
  python -m xiar_hermes.paperclip_tools --list      print the tool names
"""
from __future__ import annotations
import json, os, pathlib, selectors, subprocess, sys, threading, time, tomllib

HOME = pathlib.Path.home()
LAUNCHER = os.environ.get("XIAR_PAPERCLIP_LAUNCHER", "/opt/xiar/paperclip/xiar-paperclip-mcp")
SERVER_ROOT = HOME / ".local/share/xiar/paperclip-mcp"
CATALOG = SERVER_ROOT / "xiar-tools.json"
LOG = HOME / ".local/state/xiar/paperclip-tools.jsonl"
PROTOCOL = "2025-06-18"


def server_version() -> str:
    return os.path.basename(os.path.realpath(SERVER_ROOT / "current"))


def settings() -> dict:
    root = pathlib.Path(os.environ.get("XIAR_ROOT", str(pathlib.Path.home()/"xiar-os")))
    try:
        cfg = tomllib.loads((root / "config/paperclip_tools.toml").read_text())
    except (OSError, ValueError):
        cfg = {}
    return {"deny": [str(x) for x in cfg.get("deny", [])],
            "timeout_s": max(5, min(int(cfg.get("timeout_s", 120)), 1800)),
            "log_calls": bool(cfg.get("log_calls", True))}


class Server:
    """One Paperclip tool server per Hermes process, started on first use and restarted if it dies or hangs."""

    def __init__(self):
        self.proc = None
        self.version = None
        self.buf = b""
        self.next_id = 0
        self.lock = threading.Lock()

    def _send(self, msg: dict) -> None:
        self.proc.stdin.write((json.dumps(msg) + "\n").encode())
        self.proc.stdin.flush()

    def _read_line(self, deadline: float) -> dict:
        sel = selectors.DefaultSelector()
        sel.register(self.proc.stdout, selectors.EVENT_READ)
        try:
            while b"\n" not in self.buf:
                left = deadline - time.monotonic()
                if left <= 0 or not sel.select(left):
                    raise TimeoutError("Paperclip did not answer in time")
                chunk = os.read(self.proc.stdout.fileno(), 65536)
                if not chunk:
                    raise ConnectionError("Paperclip's tool server stopped")
                self.buf += chunk
        finally:
            sel.close()
        line, self.buf = self.buf.split(b"\n", 1)
        line = line.strip()
        return json.loads(line) if line else {}

    def _rpc(self, method: str, params: dict, timeout: float) -> dict:
        self.next_id += 1
        rid = self.next_id
        self._send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        deadline = time.monotonic() + timeout
        while True:
            msg = self._read_line(deadline)
            if msg.get("id") == rid and ("result" in msg or "error" in msg):
                if "error" in msg:
                    raise RuntimeError(str(msg["error"].get("message", msg["error"]))[:500])
                return msg["result"]
            if msg.get("method") == "ping" and "id" in msg:   # the server may check we are alive
                self._send({"jsonrpc": "2.0", "id": msg["id"], "result": {}})

    def _start(self, timeout: float) -> None:
        self.stop()
        self.version = server_version()
        self.proc = subprocess.Popen([LAUNCHER], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, start_new_session=True)
        self.buf = b""
        self._rpc("initialize", {"protocolVersion": PROTOCOL, "capabilities": {},
                                 "clientInfo": {"name": "xiar-hermes", "version": "0.1.0"}}, timeout)
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def stop(self) -> None:
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                pass
        self.proc = None

    def call(self, method: str, params: dict, timeout: float) -> dict:
        with self.lock:
            for attempt in (1, 2):
                try:
                    if self.proc is not None and self.version != server_version():
                        self.stop()   # Paperclip was updated: the next call uses the new tool server, no Hermes restart (t154u)
                    if self.proc is None or self.proc.poll() is not None:
                        self._start(min(timeout, 60))
                    return self._rpc(method, params, timeout)
                except ConnectionError:
                    self.stop()
                    if attempt == 2:
                        raise
                except Exception:
                    self.stop()      # a hung or confused server is never reused
                    raise


SERVER = Server()


def list_tools(timeout: float = 120) -> list[dict]:
    tools, cursor = [], None
    while True:
        res = SERVER.call("tools/list", {"cursor": cursor} if cursor else {}, timeout)
        tools += res.get("tools", [])
        cursor = res.get("nextCursor")
        if not cursor:
            return tools


def refresh() -> int:
    """Rebuild the tool list from the running server version (called after every Paperclip update)."""
    version = server_version()
    tools = list_tools()
    try:
        SERVER.stop()
    except Exception:
        pass
    keep = [{"name": t["name"], "description": t.get("description", ""),
             "inputSchema": t.get("inputSchema") or {"type": "object", "properties": {}}} for t in tools if t.get("name")]
    tmp = CATALOG.with_suffix(".tmp")
    tmp.write_text(json.dumps({"version": version, "tools": keep}, indent=1))
    os.replace(tmp, CATALOG)
    return len(keep)


def _log(entry: dict) -> None:
    try:
        LOG.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(LOG, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        with os.fdopen(fd, "a") as f:
            f.write(json.dumps(entry) + "\n")
    except OSError:
        pass


def _text(result: dict) -> str:
    parts = []
    for c in result.get("content", []) or []:
        parts.append(c.get("text", "") if c.get("type") == "text" else json.dumps(c))
    if result.get("structuredContent") is not None and not parts:
        parts.append(json.dumps(result["structuredContent"], ensure_ascii=False))
    return "\n".join(parts)


def make_handler(tool: str):
    def handler(args, **_kw) -> str:
        cfg = settings()
        if tool in cfg["deny"]:
            return json.dumps({"error": f"{tool} is turned off in XIAR's Paperclip tool settings"})
        t0 = time.monotonic()
        ok = False
        try:
            res = SERVER.call("tools/call", {"name": tool, "arguments": dict(args or {})}, cfg["timeout_s"])
            ok = not res.get("isError")
            out = _text(res)
            return out if ok else json.dumps({"error": out[:4000] or "Paperclip reported an error"})
        except Exception as e:
            return json.dumps({"error": f"Paperclip tool failed: {str(e)[:500]}"})
        finally:
            if cfg["log_calls"]:
                _log({"ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "tool": tool,
                      "args": sorted((args or {}).keys()), "ok": ok, "ms": int((time.monotonic() - t0) * 1000)})
    return handler


def load_catalog() -> list[dict]:
    try:
        data = json.loads(CATALOG.read_text())
    except (OSError, ValueError):
        return []
    if data.get("version") != os.path.basename(os.path.realpath(SERVER_ROOT / "current")):
        return []   # stale list after an update: nothing is offered until the refresh has run
    return data.get("tools", [])


def register(ctx) -> int:
    """Called by the XIAR plugin for Xiar only."""
    deny = set(settings()["deny"])
    n = 0
    for t in load_catalog():
        if t["name"] in deny:
            continue
        name = t["name"] if t["name"].startswith("paperclip") else "paperclip_" + t["name"]
        desc = "Paperclip: " + (t.get("description") or t["name"])
        schema = {"name": name, "description": desc, "parameters": t["inputSchema"]}
        if ctx.register_tool(name=name, toolset="paperclip", schema=schema,
                             handler=make_handler(t["name"]), description=desc) is not None:
            n += 1
    return n


if __name__ == "__main__":
    if "--refresh" in sys.argv:
        print(f"Paperclip tools: {refresh()} listed")
    elif "--list" in sys.argv:
        print("\n".join(t["name"] for t in load_catalog()))
    else:
        sys.exit(__doc__)