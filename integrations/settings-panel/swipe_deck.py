# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Settings bridge commands for the Swipe Deck panel (Menu > Panels > Swipe Deck): the cards to judge, hearts and skulls, the score, the judges and the ranking.
The ledger, the ranking and the bots' judging are integrations/hermes-xiar/xiar_hermes/swipe_deck.py and swipe_judge.py (the same files the Hermes plug-in writes cards with)."""
import importlib.util, os, pathlib, threading, time

_mods = {}


def _load(root, name, rel):
    if name not in _mods:
        spec = importlib.util.spec_from_file_location(name, root / rel)
        _mods[name] = importlib.util.module_from_spec(spec); spec.loader.exec_module(_mods[name])
    return _mods[name]


def _deck(root):
    m = _load(root, 'xiar_swipe_deck', 'integrations/hermes-xiar/xiar_hermes/swipe_deck.py')
    return m.Deck(os.environ.get('XIAR_STATE_DIR') or '~/.local/state/xiar')


def _judge(root):
    return _load(root, 'xiar_swipe_judge', 'integrations/hermes-xiar/xiar_hermes/swipe_judge.py')


def catalogue(root):
    """The 99 models (number, name, family, when to use), from the Brain's Index."""
    return [{k: m[k] for k in ('n', 'name', 'group', 'when')} for m in _judge(root).load_catalogue(root)]


def call(root, command, args=None):
    args = args or {}
    d = _deck(root)
    j = _judge(root)
    if command == 'swipe_state':
        names = {m['n']: m['name'] for m in catalogue(root)}
        card = d.next_card('owner')
        if card:
            card = dict(card, names=[names.get(n, '#%d' % n) for n in card['models']], sealed=sorted(d.bots_on(card['id'])))   # which bots have judged; what they said stays sealed
        hist = [dict(h, names=[names.get(n, '#%d' % n) for n in h['models']]) for h in d.history(8)]
        return {'stats': d.stats(), 'card': card, 'history': hist, 'settings': d.settings(), 'ranks': {str(k): v for k, v in d.ranks().items()}, 'models': catalogue(root)}
    if command == 'swipe_vote':
        # only the owner votes here (bots judge blind through the ledger); the bots' sealed opinions are revealed once the owner's vote is in
        pick = str(args.get('pick', ''))
        out = d.vote(pick, args.get('value'), 'owner', args.get('model'))
        bots = d.bots_on(pick)
        return {'voted': [out], 'reveal': {k: ('heart' if v > 0 else 'skull') for k, v in bots.items()}, 'agreed': [k for k, v in bots.items() if v == out['value']], 'stats': d.stats()}
    if command == 'swipe_note':
        n = int(args.get('n', 0))
        m = next((x for x in j.load_catalogue(root) if x['n'] == n), None)
        if m is None: raise ValueError('no such mental model')
        return {'n': n, 'name': m['name'], 'group': m['group'], 'when': m['when'], 'note': j.read_note(root, n)}
    if command == 'swipe_judges':
        return {'settings': d.settings(), 'local_now': j.resolve(root, d, 'local'), 'connections': j.connections(root)}
    if command == 'swipe_judges_set':
        role = str(args.get('role', ''))
        d.set_setting(role, args.get('value') if role == 'owner' else (str(args.get('connection', '')) or None))
        if role != 'owner': sweep(root)       # a bot that was just switched on starts on the waiting cards
        return {'settings': d.settings()}
    if command == 'swipe_judge_now':
        done = j.judge_pending(root, d, int(args['limit']) if args.get('limit') else None)
        return {'judged': {k: sorted(v) for k, v in done.items()}}
    raise ValueError('unknown swipe command')


# ---- the bots that are on go through every waiting card in the background, whether or not the panel is open
_sweeping = threading.Event()


def sweep(root):
    if _sweeping.is_set(): return
    _sweeping.set()
    def run():
        try: _judge(root).judge_pending(root, _deck(root))
        except Exception: pass
        finally: _sweeping.clear()
    threading.Thread(target=run, daemon=True).start()


def start_sweeper(root, every=60):
    def loop():
        while True:
            time.sleep(every)
            try:
                d = _deck(root)
                if any(d.pending_for(r) for r in ('cloud', 'local')): sweep(root)
            except Exception: pass
    threading.Thread(target=loop, daemon=True).start()
