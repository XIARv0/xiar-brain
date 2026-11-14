# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Native Paperclip tools (t150u): stdlib MCP stdio client against a fake server; deny list; log has no values."""
import json, os, pathlib, sys, tempfile, textwrap, unittest

FAKE = textwrap.dedent('''
    import json, sys
    for line in sys.stdin:
        m = json.loads(line)
        if "id" not in m: continue
        meth = m["method"]
        if meth == "initialize": r = {"protocolVersion": "2025-06-18", "capabilities": {}, "serverInfo": {"name": "fake"}}
        elif meth == "tools/list":
            if m["params"].get("cursor"): r = {"tools": [{"name": "connections_search", "inputSchema": {"type": "object"}}]}
            else: r = {"tools": [{"name": "paperclipEcho", "description": "echo", "inputSchema": {"type": "object", "properties": {"x": {"type": "string"}}}}], "nextCursor": "2"}
        elif meth == "tools/call":
            a = m["params"]["arguments"]
            if a.get("sleep"): import time; time.sleep(9)
            r = {"content": [{"type": "text", "text": json.dumps({"echo": a.get("x")})}], "isError": bool(a.get("fail"))}
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": r}) + "\\n"); sys.stdout.flush()
''')


class T(unittest.TestCase):
    def setUp(self):
        self.d = pathlib.Path(tempfile.mkdtemp())
        srv = self.d / "fake.py"; srv.write_text(FAKE)
        launcher = self.d / "launch"; launcher.write_text(f"#!/bin/sh\nexec {sys.executable} {srv}\n"); launcher.chmod(0o755)
        (self.d / "root/config").mkdir(parents=True)
        (self.d / "share/7").mkdir(parents=True); os.symlink("7", self.d / "share/current")
        os.environ.update(XIAR_PAPERCLIP_LAUNCHER=str(launcher), XIAR_ROOT=str(self.d / "root"))
        sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
        import importlib
        import xiar_hermes.paperclip_tools as pt
        self.pt = importlib.reload(pt)
        self.pt.SERVER_ROOT = self.d / "share"; self.pt.CATALOG = self.d / "share/xiar-tools.json"; self.pt.LOG = self.d / "log.jsonl"

    def tearDown(self):
        self.pt.SERVER.stop()

    def ctx(self):
        class C:
            t = {}
            def register_tool(s, name, toolset, schema, handler, description=""):
                s.t[name] = (schema, handler); return object()
        return C()

    def test_refresh_register_call_and_log(self):
        self.assertEqual(self.pt.refresh(), 2)
        c = self.ctx(); self.assertEqual(self.pt.register(c), 2)
        self.assertIn("paperclip_connections_search", c.t)
        out = c.t["paperclipEcho"][1]({"x": "secret-value"})
        self.assertEqual(json.loads(out), {"echo": "secret-value"})
        log = self.pt.LOG.read_text()
        self.assertNotIn("secret-value", log); self.assertIn('"args": ["x"]', log)
        self.assertEqual(oct(self.pt.LOG.stat().st_mode & 0o777), "0o600")

    def test_error_and_deny_and_stale(self):
        self.pt.refresh(); c = self.ctx(); self.pt.register(c)
        self.assertIn("error", json.loads(c.t["paperclipEcho"][1]({"fail": True})))
        (self.d / "root/config/paperclip_tools.toml").write_text('deny = ["paperclipEcho"]\n')
        self.assertIn("turned off", c.t["paperclipEcho"][1]({}))
        c2 = self.ctx(); self.assertEqual(self.pt.register(c2), 1)
        os.remove(self.d / "share/current"); os.symlink("8", self.d / "share/current")
        self.assertEqual(self.pt.register(self.ctx()), 0)   # stale list after an update offers nothing

    def test_timeout_then_recovers(self):
        self.pt.refresh(); c = self.ctx(); self.pt.register(c)
        (self.d / "root/config/paperclip_tools.toml").write_text("timeout_s = 5\n")
        self.assertIn("did not answer", c.t["paperclipEcho"][1]({"sleep": True}))
        self.assertEqual(json.loads(c.t["paperclipEcho"][1]({"x": "a"})), {"echo": "a"})


if __name__ == "__main__":
    unittest.main()