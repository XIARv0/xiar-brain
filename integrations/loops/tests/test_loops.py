# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
import json, pathlib, sys, tempfile, time, unittest
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import xiar_loops as L


def cfg(tmp, **wake):
    c = L.load(); d = pathlib.Path(tmp)
    c['paths'].update(state_dir=str(d), events_log=str(d / 'events.jsonl'), status_file=str(d / 'status.json'), identity_block=str(d / 'id.txt'), autonomy_state=str(d / 'state.json'))
    c['wake'].update(wake); return c


def s(**k): return dict(dict(load_ratio=0.1, mem_ratio=0.3, disk_ratio=0.4, disk_used_mb=1000, temp_c=40.0, window_class='term'), **k)


class Reflex(unittest.TestCase):
    def setUp(self): self.t = tempfile.TemporaryDirectory(); self.c = cfg(self.t.name, mode='xiar')
    def tearDown(self): self.t.cleanup()

    def test_quiet_machine_backs_off_to_the_maximum(self):
        e = L.Engine(self.c); waits = [e.step(s(), 100 + i)[1] for i in range(12)]
        self.assertEqual(waits[-1], self.c['reflex']['interval_max_s']); self.assertTrue(all(b >= a for a, b in zip(waits, waits[1:])))

    def test_only_a_change_triggers_and_resets_the_interval(self):
        e = L.Engine(self.c); e.step(s(), 1)
        trig, wait = e.step(s(load_ratio=0.99), 2); self.assertEqual(trig, ['cpu']); self.assertEqual(wait, self.c['reflex']['interval_s'])
        self.assertIsNone(e.step(s(load_ratio=0.99), 3)[0], 'the same condition staying true is not a new change')
        self.assertIsNone(e.step(s(), 4)[0]); self.assertEqual(e.step(s(load_ratio=0.99), 5)[0], ['cpu'])

    def test_disk_growth_and_focus_churn(self):
        e = L.Engine(self.c); e.step(s(), 1)
        self.assertEqual(e.step(s(disk_used_mb=1000 + 4096), 2)[0], ['disk-growth'])
        e2 = L.Engine(self.c); got = None
        for i in range(10):
            r = e2.step(s(window_class='a' if i % 2 else 'b'), 10 + i)[0]; got = got or r
        self.assertEqual(got, ['focus-churn'])

    def test_emergency_stop_and_log_mode_never_wake(self):
        e = L.Engine(self.c); self.assertEqual(e.may_wake(1000, True), (False, 'stopped'))
        self.assertEqual(L.Engine(cfg(self.t.name, mode='log')).may_wake(1000, False), (False, 'log-only'))

    def test_kill_switch_caps_wakes_per_hour(self):
        e = L.Engine(cfg(self.t.name, mode='xiar', cooldown_s=0, max_wakes_per_hour=3))
        self.assertEqual([e.may_wake(1000 + i, False)[0] for i in range(5)], [True, True, True, False, False])
        self.assertEqual(e.may_wake(1000 + 3600 * 2, False)[0], True, 'a new hour starts clean')

    def test_cooldown(self):
        e = L.Engine(cfg(self.t.name, mode='xiar', cooldown_s=300)); self.assertTrue(e.may_wake(1000, False)[0]); self.assertEqual(e.may_wake(1100, False), (False, 'cooldown'))

    def test_missing_state_file_counts_as_stopped(self):
        self.assertTrue(L.stopped(self.c))


class Loops(unittest.TestCase):
    def setUp(self): self.t = tempfile.TemporaryDirectory(); self.c = cfg(self.t.name)
    def tearDown(self): self.t.cleanup()

    def test_circadian_boredom_backoff_then_digest(self):
        self.assertEqual(L.circadian(self.c)['next_skip'], 1)       # nothing happened
        self.assertEqual(L.circadian(self.c), dict(skipped=True))   # the owed run is skipped
        self.assertEqual(L.circadian(self.c)['next_skip'], 2)       # quiet again: stretch
        L.log_event(self.c, 'trigger', conditions=['cpu'], wake=False, reason='log-only')
        L.circadian(self.c); L.circadian(self.c)                    # burn the two owed skips
        out = L.circadian(self.c); self.assertEqual((out['triggers'], out['conditions']), (1, {'cpu': 1}))

    def test_dream_prunes_old_and_duplicate_triggers(self):
        p = pathlib.Path(self.c['paths']['events_log']); now = time.time(); old = now - 40 * 86400
        rows = [dict(t=old, kind='trigger', conditions=['cpu']), dict(t=now - 5, kind='trigger', conditions=['mem']), dict(t=now - 4, kind='trigger', conditions=['mem']), dict(t=now - 3, kind='circadian-quiet')]
        p.write_text('\n'.join(json.dumps(r) for r in rows) + '\n')
        out = L.dream(self.c); self.assertEqual(out['before'], 4)
        kept = [r for r in L.read_events(self.c) if r['kind'] == 'trigger']
        self.assertEqual([r['conditions'] for r in kept], [['mem']])


if __name__ == '__main__':
    unittest.main()
