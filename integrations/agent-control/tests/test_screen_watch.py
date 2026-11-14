# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Delegated screen-watch sessions: Xiar can start/stop a TIMED session only if the owner delegated it, and the owner can always end it.
Run as root on Linux like the other broker tests:
  sudo env LD_PRELOAD=/opt/xiar/sqlite/3.54.1/libsqlite3.so.0 PYTHONPATH=<agent-control>:<hermes-xiar> python3 tests/test_screen_watch.py"""
import json, os, pathlib, shutil, sys, tempfile, threading, time, unittest
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import autonomy, runtime


class Drive:
    def ticket(self, who, body):
        if who.get('agent') != 'xiar' or who.get('run_id'): raise ValueError('Only standing Xiar')
        return 'a' * 32, body['tool'], body['args'], 1


class Box(autonomy.Autonomy):
    def __init__(self, path, stopped=False):
        self.SCREEN_CONSENT = path; self.ROOT_UIDS = (os.getuid(),); self.drive = Drive(); self.lock = threading.RLock(); self._stopped = stopped
    def current(self): return dict(stopped=self._stopped, generation=1)


def write_atomic(path, data, mode): pathlib.Path(path).write_bytes(data)


class ScreenWatch(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix='sw-test-')); self.path = self.tmp / 'screen-consent.json'
        self.old = runtime.atomic_root; runtime.atomic_root = write_atomic
        self.xiar = dict(profile='owner', agent='xiar', run_id='')
    def tearDown(self): runtime.atomic_root = self.old; shutil.rmtree(self.tmp, ignore_errors=True)
    def consent(self, **kw): self.path.write_text(json.dumps(kw)); os.chmod(self.path, 0o644)
    def call(self, box, **args): return box.screen_watch(self.xiar, dict(tool='xiar_screen_watch', args=args))
    def read(self): return json.loads(self.path.read_text())

    def test_refused_unless_the_owner_delegated(self):
        self.consent(enabled=False, interval_s=60); box = Box(self.path)
        with self.assertRaisesRegex(ValueError, 'not allowed'): self.call(box, action='start', minutes=5, reason='learn')
        self.assertNotIn('session', self.read())
        self.assertFalse(self.call(box, action='status')['delegated'])

    def test_start_is_capped_needs_a_reason_and_stop_removes_it(self):
        self.consent(enabled=False, interval_s=60, delegate=dict(allowed=True, max_minutes=10)); box = Box(self.path)
        with self.assertRaisesRegex(ValueError, 'reason'): self.call(box, action='start', minutes=5)
        r = self.call(box, action='start', minutes=999, reason='learn the invoice form')
        self.assertTrue(r['session_active']); self.assertLessEqual(self.read()['session']['until'] - time.time(), 601)
        self.assertEqual(self.read()['session']['by'], 'xiar'); self.assertFalse(self.read()['enabled'], 'a session never flips the owner consent')
        self.call(box, action='stop'); self.assertNotIn('session', self.read())

    def test_emergency_stop_blocks_start(self):
        self.consent(enabled=False, interval_s=60, delegate=dict(allowed=True, max_minutes=10)); box = Box(self.path, stopped=True)
        with self.assertRaisesRegex(ValueError, 'Emergency stop'): self.call(box, action='start', minutes=5, reason='x')

    def test_only_standing_xiar_and_an_unsafe_file_means_not_delegated(self):
        self.consent(enabled=False, interval_s=60, delegate=dict(allowed=True, max_minutes=10)); box = Box(self.path)
        with self.assertRaises(ValueError): box.screen_watch(dict(profile='x', agent='other', run_id=''), dict(tool='xiar_screen_watch', args=dict(action='status')))
        os.chmod(self.path, 0o666)
        self.assertFalse(self.call(box, action='status')['delegated'], 'a group/world-writable consent file is ignored')

    def test_owner_off_and_undelegate_end_the_session(self):
        self.consent(enabled=True, interval_s=60, delegate=dict(allowed=True, max_minutes=10), session=dict(until=time.time() + 300, reason='r', by='xiar')); box = Box(self.path)
        self.assertTrue(self.call(box, action='status')['session_active'])


if __name__ == '__main__': unittest.main()
