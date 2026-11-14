# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Blind bot judging: what a judge is sent, the default local judge, who goes through which cards, and that failure means no vote."""
import pathlib, sys, pytest
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from xiar_hermes import swipe_judge as sj
from xiar_hermes.swipe_deck import Deck

ROOT = pathlib.Path(__file__).resolve().parents[3]


@pytest.fixture
def deck(tmp_path): return Deck(tmp_path)


def test_the_catalogue_and_a_note_come_from_the_index():
    cat = sj.load_catalogue(ROOT); assert len(cat) == 99 and cat[0]['rel'] and cat[0]['when']
    assert len(sj.read_note(ROOT, 7)) > 200 and len(sj.read_note(ROOT, 7, 100)) <= 100 and sj.read_note(ROOT, 1000) == ''


def test_what_a_judge_is_sent_by_kind():
    models = [{"name": "Inversion", "when": "Think backwards.", "note": "Invert the problem."}]
    sug = sj._messages({"kind": "suggest", "task": "raise prices?"}, models)[1]["content"]
    out = sj._messages({"kind": "outcome", "task": "raise prices?", "answer": "Raise them 5%."}, models)[1]["content"]
    assert "Invert the problem." in sug and "raise prices?" in sug and "Raise them" not in sug
    assert "Raise them 5%." in out and "Did the model lead to a good decision" in out
    for text in (sug, out): assert "owner" not in text.lower() and "vote" not in text.lower()


def test_local_is_on_by_default_and_cloud_needs_a_choice(deck, monkeypatch):
    monkeypatch.setattr(sj, "connections", lambda root: [{"id": "big-cloud", "where": "provider"}, {"id": "nano", "where": "here"}])
    assert deck.enabled_bots() == ["local"] and sj.resolve(ROOT, deck, "local") == "nano" and sj.resolve(ROOT, deck, "cloud") is None
    deck.set_setting("cloud", "big-cloud"); assert deck.enabled_bots() == ["cloud", "local"] and sj.resolve(ROOT, deck, "cloud") == "big-cloud"
    deck.set_setting("local", ""); assert deck.enabled_bots() == ["cloud"]


def test_a_bot_goes_through_every_waiting_card_and_the_ledger_keeps_the_order(deck, monkeypatch):
    monkeypatch.setattr(sj, "connections", lambda root: [{"id": "nano", "where": "here"}])
    asked = []
    monkeypatch.setattr(sj, "ask", lambda root, conn, pick, models: asked.append((conn, pick["id"])) or 1)
    ids = [deck.record_pick("xiar", "t%d" % i, [7], [.9]) for i in range(3)]
    deck.vote(ids[2], "heart", "owner")                                      # the owner got there first on one card: the bot still judges it, blind (t134u)
    out = sj.judge_pending(ROOT, deck)
    assert sorted(out) == sorted(ids) and [a[1] for a in asked] == ids and all(v == {"local": 1} for v in out.values())
    assert sj.judge_pending(ROOT, deck) == {}                                # nothing waiting now


def test_no_opinion_is_never_made_up(deck, monkeypatch):
    monkeypatch.setattr(sj, "connections", lambda root: [{"id": "nano", "where": "here"}]); monkeypatch.setattr(sj, "ask", lambda *a: None)
    pid = deck.record_pick("xiar", "t", [7], [.9])
    sj.judge_pending(ROOT, deck); sj.judge_pending(ROOT, deck)
    assert deck.bots_on(pid) == {} and deck.pending_for("local") == [] and deck.next_card()["id"] == pid        # skipped after two failures, so the owner is not stuck


def test_one_word_answers_only(monkeypatch):
    class M:
        def connections(self, rt): return {"c": {"kind": "chat", "type": "local"}}
        def request(self, c, path, body): return {"choices": [{"message": {"content": M.text}}]}
    monkeypatch.setattr(sj, "_adapter", lambda root: M()); monkeypatch.setattr(sj.tomllib, "loads", lambda s: {}); monkeypatch.setattr(pathlib.Path, "read_text", lambda self, **k: "")
    for text, want in (("HEART", 1), ("**Skull**", -1), ("skull.", -1), ("Maybe heart or skull", None), ("", None)):
        M.text = text; assert sj.ask("/x", "c", {"kind": "use", "task": "t"}, [{"name": "a"}]) == want, text
