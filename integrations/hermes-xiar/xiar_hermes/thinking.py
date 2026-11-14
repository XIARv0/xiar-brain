# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Job 6: score every mirrored mental model with resident Laya, then inject top-k.

The catalogue and note text remain unchanged. The prior adapter's top three, every
turn (independent of model thinking) are retained as configurable behaviour.
No lexical fallback is used by this job when Laya fails or abstains.
"""
from __future__ import annotations
from .decisions import Audit, score
from .swipe_deck import Deck
import random
import threading

LEVELS = ["not relevant", "slightly relevant", "somewhat useful", "very useful", "directly applicable"]

class ThinkingModels:
    def __init__(self, cfg, laya, catalogue):
        self.cfg, self.laya, self.catalogue = cfg, laya, catalogue
        self.audit = Audit(cfg)
        self.deck = Deck(cfg.get("routing.state_dir", "~/.local/state/xiar"))
        self.explore = max(0, int(cfg.get("mental_models.explore", 2)))
        self.top = max(0, int(cfg.get("mental_models.inject_top", 3)))
        self.batch = max(1, min(32, int(cfg.get("mental_models.batch_size", 8))))
        self.minimum = float(cfg.get("mental_models.confidence_min", 0.6))
        self.levels = list(cfg.get("mental_models.score_levels", LEVELS))
        if len(self.levels) < 2:
            raise ValueError("mental model scoring requires at least two levels")
        self.max_chars = int(cfg.get("mental_models.max_query_chars", 1500))
        self.families_top = max(1, int(cfg.get("mental_models.families_top", 2)))
        self.timeout = float(cfg.get("mental_models.laya_timeout_s", 20))   # one batch of 8 takes ~1.7 s on the VM CPU and Laya serves one job at a time: the 5 s default fails whenever it is busy

    def _decide(self, text, questions):
        """One batch: a longer wait than the other jobs get (this one is not on a hot path), and one retry."""
        http = getattr(self.laya, "http", None)
        old = getattr(http, "timeout", None)
        if old is not None:
            http.timeout = max(old, self.timeout)
        try:
            for _ in range(2):
                answers = self.laya.decide(text, questions)
                if answers is not None:
                    return answers
            return None
        finally:
            if old is not None:
                http.timeout = old

    def families(self, text):
        """Step 1 of the funnel: Laya scores each FAMILY of models (its name and a few of its models' names), and the best `families_top` are kept."""
        models = self.catalogue.models if self.catalogue else []
        groups = list(dict.fromkeys(m.group for m in models))
        scored = []
        for start in range(0, len(groups), self.batch):
            batch = groups[start:start + self.batch]
            questions = [{"type": "score", "instructions": "How useful is this family of mental models for the user's task? %s: %s" % (
                g, ", ".join(m.name for m in models if m.group == g)[:150]), "levels": self.levels} for g in batch]
            answers = self._decide(text[:self.max_chars], questions)
            if answers is None:
                raise ValueError("Laya unavailable")
            scored.extend((score(a, len(self.levels)), g) for g, a in zip(batch, answers))
        ranks = self.deck.ranks()          # hearts and skulls nudge a family by the average of its models' factors (1.0 until they have votes)
        def nudge(g):
            fs = [self.deck.factor(m.num, ranks) for m in models if m.group == g]
            return sum(fs) / len(fs) if fs else 1.0
        scored = [(p * nudge(g), g) for p, g in scored]
        scored.sort(key=lambda pair: -pair[0])
        return [g for _, g in scored[:self.families_top]]

    def ranked(self, text, agent_id=None):
        """Laya decides from ALL the models, in two steps so it stays fast: the best families first, then every model in them (plus a few from the others, so none is frozen out).
        Nobody's hand-picked list narrows this; hearts and skulls only nudge the scores."""
        ranked = []
        models = self.catalogue.models if self.catalogue else []
        top = self.families(text) if models else []
        self.last_families = top
        outside = [m for m in models if m.group not in top]
        keep = {m.num for m in models if m.group in top} | {m.num for m in random.sample(outside, min(self.explore, len(outside)))}
        models = [m for m in models if m.num in keep]
        for start in range(0, len(models), self.batch):
            batch = models[start:start + self.batch]
            questions = [{"type": "score", "instructions":
                f"How useful is this mental model for the user's task? {m.name}: {m.when}",
                "levels": self.levels} for m in batch]
            answers = self._decide(text[:self.max_chars], questions)
            if answers is None:
                raise ValueError("Laya unavailable")
            ranked.extend((score(a, len(self.levels)), m) for m, a in zip(batch, answers))
        ranks = self.deck.ranks()         # hearts and skulls nudge the score (0.75x to 1.25x) once a model has enough votes
        ranked = [(min(1.0, p * self.deck.factor(m.num, ranks)), m) for p, m in ranked]
        return sorted(ranked, key=lambda pair: (-pair[0], pair[1].num))

    def select(self, text, agent_id=None):
        if not text.strip() or self.top == 0:
            return []
        try:
            ranked = self.ranked(text, agent_id)
            selected = [(p, m) for p, m in ranked if p >= self.minimum][:self.top]
            self.audit.write(6, "above_threshold" if selected else "below_threshold",
                threshold=self.minimum, evaluated=len(ranked), families=getattr(self, "last_families", []), selected=[m.num for _, m in selected],
                scores=[round(p, 4) for p, _ in selected])
            if selected:
                self.last_pick = self.deck.record_pick(agent_id or "xiar", text, [m.num for _, m in selected], [p for p, _ in selected])
                self.sweep_in_background()
            return [m for _, m in selected]
        except (ValueError, TypeError, KeyError) as e:
            self.audit.write(6, "unavailable_or_invalid", evaluated=0, error=str(e)[:80])   # why: "Laya unavailable" (no answer) or an invalid answer
            return []

    def sweep_in_background(self):
        """The bots that are on (the owner's choice) go through every waiting card, off the reply path. One sweep at a time; no bot on or any failure means no opinion."""
        if getattr(self, "_sweeping", False) or not self.cfg.get("mental_models.judge", True):
            return
        self._sweeping = True

        def run():
            try:
                from . import swipe_judge
                swipe_judge.judge_pending(self.cfg.root, self.deck)
            except Exception:
                pass
            finally:
                self._sweeping = False
        threading.Thread(target=run, daemon=True).start()
