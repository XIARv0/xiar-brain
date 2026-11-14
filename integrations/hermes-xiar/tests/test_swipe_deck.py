# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Swipe Deck ledger: the points, the three kinds of card, who judges, blind order, completion, ranking and the funnel hookup into job 6."""
import copy, pathlib, sys, pytest
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from xiar_hermes import swipe_deck as sd
from xiar_hermes.swipe_deck import Deck
from xiar_hermes.config import load, AdapterConfig
from xiar_hermes.mental_models import MentalModels
from xiar_hermes.thinking import ThinkingModels
from test_jobs654 import Laya, ROOT


@pytest.fixture
def deck(tmp_path):
    return Deck(tmp_path)


def test_the_points():
    assert sd.WEIGHT == {"owner": 3.0, "cloud": 2.0, "local": 2.0, "agent": 1.0, "laya": 1.0, "auto": 1.0}


def test_three_kinds_of_card_and_the_agent_and_laya_points(deck):
    s = deck.record_pick('xiar', 'raise prices?', [7, 99, 89], [.9, .8, .7])
    u = deck.record_use('xiar', 99); assert u and u != s
    assert deck.record_use('xiar', 99) == u                                   # the same use is one card
    o = deck.record_outcome('xiar', 'Raise them 5 percent, and watch churn for a month.'); assert o
    assert deck.record_outcome('xiar', 'again') is None                       # one outcome per use
    kinds = {p['id']: p['kind'] for p in deck.picks()}; assert kinds == {s: 'suggest', u: 'use', o: 'outcome'}
    votes = deck._read('votes.jsonl'); assert {(v['pick'], v['voter']) for v in votes} == {(s, 'laya'), (u, 'agent')}
    assert deck.record_use('someone else', 5) is None and deck.record_outcome('someone else', 'x') is None
    r = deck.ranks(); assert r[99]['hearts'] == 2.0 and r[7]['hearts'] == 1.0 and r[89]['hearts'] == 1.0   # laya 1 for each offered model, plus the agent's 1 for its choice


def test_ranking_waits_for_enough_weight_then_nudges(deck):
    pid = deck.record_pick('xiar', 't', [5, 10, 52], [.9, .8, .7])
    assert deck.factor(5) == 1.0                                              # laya's 1 point alone moves nothing
    deck.vote(pid, 'heart', 'cloud'); assert deck.factor(5) > 1.0              # 1 + 2 = 3 points
    deck.vote(pid, 'skull', 'owner', model=52); assert deck.factor(52) < deck.factor(5) <= 1.25 and deck.factor(52) >= 0.75


def test_refuses_bad_votes(deck):
    pid = deck.record_pick('xiar', 't', [5], [.9])
    for args in (('nope', 1, 'owner'), (pid, 2, 'owner'), (pid, 1, 'stranger')):
        with pytest.raises(ValueError): deck.vote(*args)
    with pytest.raises(ValueError): deck.vote(pid, 1, 'owner', model=99)
    deck.vote(pid, 1, 'owner')
    with pytest.raises(ValueError): deck.vote(pid, 1, 'owner')


def test_bots_judge_blind_and_the_owner_gets_the_card_at_the_same_time(deck):
    deck.set_setting('cloud', 'c1')                                           # local is on by default, cloud was just switched on
    pid = deck.record_pick('xiar', 'x', [5], [.9])
    assert deck.next_card()['id'] == pid and deck.stats()['waiting_for_bots'] == 1 and deck.settings()['local'] == 'default'   # the owner has it at once (t134u)
    assert [p['id'] for p in deck.pending_for('cloud')] == [pid] == [p['id'] for p in deck.pending_for('local')]               # and so do the bots
    deck.vote(pid, 'skull', 'cloud')
    with pytest.raises(ValueError): deck.vote(pid, 'heart', 'cloud')           # once per bot
    deck.vote(pid, 'heart', 'owner')                                          # the owner votes before the local bot
    assert deck.next_card() is None and deck.history() == [] and [p['id'] for p in deck.pending_for('local')] == [pid]   # finished only when the local bot is done
    deck.vote(pid, 'heart', 'local')
    assert deck.bots_on(pid) == {'cloud': -1, 'local': 1} and deck.history()[0]['id'] == pid and deck.stats()['waiting_for_bots'] == 0


def test_owner_may_stay_out_of_it_and_a_card_finishes_when_the_enabled_judges_are_done(deck):
    deck.set_setting('owner', False); deck.set_setting('local', ''); deck.set_setting('cloud', 'c1')
    pid = deck.record_pick('xiar', 'x', [5], [.9])
    assert deck.next_card() is None and deck.history() == [] and [p['id'] for p in deck.pending_for('cloud')] == [pid]
    deck.vote(pid, 'heart', 'cloud')
    assert [h['id'] for h in deck.history()] == [pid] and deck.pending_for('cloud') == [] and deck.stats()['finished'] == 1
    assert deck.picks()                                                       # nothing is ever deleted


def test_a_bot_that_cannot_answer_is_skipped_so_the_card_never_waits_forever(deck):
    pid = deck.record_pick('xiar', 'x', [5], [.9])                            # local is on by default
    deck.vote(pid, 'heart', 'owner')                                          # t134u: the owner has the card at once
    deck.note_failure(pid, 'local'); assert deck.history() == [] and [p['id'] for p in deck.pending_for('local')] == [pid]
    deck.note_failure(pid, 'local'); assert deck.pending_for('local') == [] and [h['id'] for h in deck.history()] == [pid]


def test_cards_combo_xp_and_agreement(deck):
    deck.set_setting('local', ''); deck.set_setting('cloud', 'c1')
    a = deck.record_pick('xiar', 'one', [5], [.9]); b = deck.record_pick('xiar', 'two', [6], [.9])
    deck.vote(a, 'heart', 'cloud'); deck.vote(b, 'skull', 'cloud')
    assert deck.next_card()['id'] == a
    deck.vote(a, 'heart', 'owner'); deck.vote(b, 'heart', 'owner')
    s = deck.stats(); assert (s['hearts'], s['skulls'], s['combo'], s['agreements'], s['xp']) == (2, 0, 2, 1, 25 + sd.QUEST_XP)   # 2 votes (20) + 1 agreement (5) + the "Clear the deck" quest (20)


def test_a_correction_gives_one_automatic_skull_to_the_latest_card(deck):
    deck.record_pick('xiar', 'x', [5], [.9]); deck.record_use('xiar', 5); o = deck.record_outcome('xiar', 'answer')
    assert deck.auto_skull_latest('xiar')['pick'] == o and deck.auto_skull_latest('xiar') is None
    assert deck.auto_skull_latest('someone else') is None


def test_laya_picks_from_all_models_and_the_pick_is_written_down(tmp_path):
    c = load(str(ROOT / 'config/hermes_adapter.toml')); raw = copy.deepcopy(c.raw); raw['routing']['state_dir'] = str(tmp_path)
    raw['laya']['jobs']['memory_routing'] = False; raw['mental_models']['dir'] = str(ROOT / 'Brain/Shared/mental-models')
    raw['mental_models']['judge'] = False
    cfg = AdapterConfig(raw, ROOT); cat = MentalModels(cfg.path('mental_models.dir'))
    l = Laya(); job = ThinkingModels(cfg, l, cat)
    a = job.select('Evaluate the deployment risks', 'board skeptic'); l2 = Laya(); b = ThinkingModels(cfg, l2, cat).select('Evaluate the deployment risks', 'no such agent')
    assert sum(map(len, l.asked)) == sum(map(len, l2.asked)) and a and b                  # nobody's hand-picked list narrows the pick: every agent gets the same funnel
    assert any(p['kind'] == 'suggest' for p in job.deck.picks())                          # Laya's pick was written down


def test_daily_quests_streak_and_bonus_xp(deck):
    import json, time
    deck.set_setting('local', '')                                             # the owner alone judges
    ids = [deck.record_pick('xiar', 't%d' % i, [5], [.9]) for i in range(5)]
    q = {x['id']: x for x in deck.stats()['quests']}; assert not q['judge5']['done'] and q['judge5']['have'] == 0 and deck.stats()['streak'] == 0
    for pid in ids: deck.vote(pid, 'heart', 'owner')
    s = deck.stats(); q = {x['id']: x for x in s['quests']}
    assert q['judge5']['done'] and q['clear']['done'] and not q['agree3']['done']
    assert s['streak'] == 1 and s['bonus_xp'] == 2 * sd.QUEST_XP and s['xp'] == 50 + 2 * sd.QUEST_XP
    # the two days before also had a vote: the streak is 3, and the best run is kept
    day = 86400
    with open(deck.dir / 'votes.jsonl', 'a') as f:
        for back in (1, 2):
            f.write(json.dumps({"pick": ids[0], "at": time.time() - back * day, "voter": "owner", "value": 1, "model": None}) + chr(10))
    assert deck.stats()['streak'] == 3 and deck.stats()['best_streak'] == 3
