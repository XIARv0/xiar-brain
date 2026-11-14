# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Job 5: Laya detects user critiques; behaviour goes directly to the sealed writer.

No approval workflow, no searchable lesson store. Facts go ONLY to tenant Semantic
memory. Existing trainer owns decryption, sparse updates, replay and promotion.
The old Rust session distiller extracts pending SkillExperience, not neural
lessons; its user_critique provenance and bounded transcript principle are kept,
not its unrelated pending-experience workflow.
"""
from __future__ import annotations
import re
from .decisions import Audit, choice, probability

KINDS = ["behaviour: correction of the assistant's style, reasoning, tool use or following instructions; no business facts",
         "fact: correction of business or client information, names, numbers, records or documents",
         "mixed: both assistant behaviour and business or client information",
         "none: not a correction"]
DOMAINS = ["presentation: response format, length or tone", "instructions: follow the owner's existing design or request",
           "tool use: an action or tool was used incorrectly", "reasoning: incorrect approach or unsupported assumption"]
Q = "Is the latest user message correcting something the assistant did or said, rather than just making a new request?"

class Corrections:
    def __init__(self, cfg, laya, brain, memory):
        self.cfg, self.laya, self.brain, self.memory = cfg, laya, brain, memory
        self.audit = Audit(cfg)
        self.minimum = float(cfg.get("corrections.confidence_min", 0.8))
        self.type_min = float(cfg.get("corrections.type_min", 0.6))
        self.max_chars = int(cfg.get("corrections.max_chars", 550))
        self.agent = str(cfg.get("identity.agent_id", "xiar"))
        self.secret_rx = [re.compile(p) for p in cfg.get("routing.never_store_patterns", [])]

    def classify(self, text, previous=""):
        a = self.laya.decide(f"Previous assistant response: {previous[:self.max_chars]}\nLatest user message: {text[:self.max_chars]}", [
            {"type": "noul", "instructions": Q},
            {"type": "choice", "instructions": "What is the latest user message correcting?", "options": KINDS},
            {"type": "choice", "instructions": "Which assistant behaviour was corrected?", "options": DOMAINS}])
        if a is None:
            raise ValueError("Laya unavailable")
        p = probability(a[0].get("probability")); kinds = choice(a[1], len(KINDS)); domains = choice(a[2], len(DOMAINS))
        k = max(range(len(kinds)), key=kinds.__getitem__); d = max(range(len(domains)), key=domains.__getitem__)
        return dict(probability=p, kind=k, kind_probability=kinds[k], domain=d, domain_probability=domains[d])

    def handle(self, text, previous="", business=""):
        if not text.strip():
            return []
        if any(rx.search(text) for rx in self.secret_rx):
            self.audit.write(5, "secret_refused")
            return []
        # Never clip an input into a pure-behaviour fragment while hiding factual context.
        # Whole-message classification happens before any text is passed to the writer.
        if len(text) > self.max_chars:
            self.audit.write(5, "over_length", chars=len(text))
            return []
        try:
            d = self.classify(text, previous)
        except (ValueError, KeyError, TypeError):
            self.audit.write(5, "unavailable_or_invalid")
            return []
        reason = "below_threshold"
        written = False
        if d["probability"] >= self.minimum and getattr(self, "on_correction", None):
            try: self.on_correction()      # the owner corrected the agent: its latest mental-model pick gets an automatic skull (Swipe Deck)
            except Exception: pass
        if d["probability"] >= self.minimum and d["kind_probability"] >= self.type_min:
            if d["kind"] == 0:
                if self.agent != "xiar":
                    reason = "only_xiar_has_brain"
                else:
                    # No previous response/tool arguments/business id are copied to weights.
                    # The brain's existing LessonPolicy remains a second deny backstop.
                    written = self.brain.lesson(dict(kind="correction", outcome="corrected",
                        action="assistant behaviour: " + DOMAINS[d["domain"]].split(":")[0],
                        context="owner user_critique", why=text, source="hermes:laya-correction"))
                    reason = "sealed_writer_accepted" if written else "sealed_writer_refused"
            elif d["kind"] in (1, 2):
                if business:
                    result = self.memory.insert(business, text[:80], text, source="hermes:laya-correction", entity_types=[])
                    written = "id" in result
                    reason = "tenant_semantic_accepted" if written else "tenant_semantic_refused"
                else:
                    reason = "business_required_no_write"
            else:
                reason = "not_correction"
        self.audit.write(5, reason, **d)
        return [dict(reason=reason, written=written, **d)]