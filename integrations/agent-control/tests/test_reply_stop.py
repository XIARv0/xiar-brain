# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""The owner's 'stop all model replies now': replies that began earlier are told to cancel, later ones are not. Run on the VM as root like the other broker tests:
  sudo env LD_PRELOAD=/opt/xiar/sqlite/3.54.1/libsqlite3.so.0 PYTHONPATH=<agent-control>:<hermes-xiar> python3 tests/test_reply_stop.py"""
import os, pathlib, shutil, sys, tempfile, threading, time, unittest
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import scheduler_store, scheduler_upstream as U

class Sched:
    def __init__(self, store): self.store = store; self.lifecycle = threading.RLock(); self.stopped = False
    def emergency(self): return dict(stopped=self.stopped, cutoff=0)

class ReplyStop(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix='rs-test-'))
        self.s = Sched(scheduler_store.Store(self.tmp)); self.u = U.Upstream(self.s)
    def tearDown(self): shutil.rmtree(self.tmp, ignore_errors=True)
    def add(self, rid, created, state='open'):
        with self.s.store.db() as db:
            db.execute('INSERT INTO model_requests VALUES(?,?,?,?,?,?,?,?,?,?,?,?)', (rid, '', 'c', 'auto', 0, 1, 'fp', state, os.getpid(), U.process_identity(os.getpid()), created, created))
    def check(self, rid): return self.u.call(dict(action='check', request_id=rid), os.getpid())

    def test_earlier_replies_are_cancelled_and_counted_later_ones_are_not(self):
        a, b, c = 'a' * 32, 'b' * 32, 'c' * 32
        self.add(a, time.time() - 5); self.add(b, time.time() - 1)
        self.assertEqual(self.check(a), dict(cancel=False))
        self.assertEqual(self.u.cancel_replies(), 2)
        self.assertEqual(self.check(a), dict(cancel=True)); self.assertEqual(self.check(b), dict(cancel=True))
        time.sleep(0.02); self.add(c, time.time())
        self.assertEqual(self.check(c), dict(cancel=False), 'a reply that begins after the button is untouched')

    def test_nothing_running_says_zero(self):
        self.assertEqual(self.u.cancel_replies(), 0)

if __name__ == '__main__':
    unittest.main()
