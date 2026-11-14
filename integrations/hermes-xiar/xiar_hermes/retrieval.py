# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Job 4: Laya chooses scoped keyword/ontology recall, brain signals, or a mix.

MIRIX retrieval reuses xiar-memory / xiar-ontology via the existing Rust API.
Brain means inference signals only, never lesson-store search. Caller identity is
bound to an API token; Laya cannot choose or override an agent/business/user.
"""
from __future__ import annotations
from .decisions import Audit, choice

NAMES = ["keyword", "ontology", "brain", "keyword+ontology", "ontology+brain"]
OPTIONS = ["keyword: exact names, phrases, identifiers or literal text in stored business facts",
    "ontology: relationships, categories, entity types and conceptual business facts",
    "brain: the assistant's learned behaviour, past mistakes and preferences; not business records",
    "keyword+ontology: both exact text and conceptual relationships in business memory",
    "ontology+brain: business relationships plus the assistant's learned behaviour"]

class Retrieval:
    def __init__(self, cfg, laya, memory, brain):
        self.cfg, self.laya, self.memory, self.brain = cfg, laya, memory, brain
        self.audit = Audit(cfg)
        self.minimum = float(cfg.get("retrieval.confidence_min", 0.6))
        self.max_chars = int(cfg.get("retrieval.max_query_chars", 1500))
        self.max_limit = int(cfg.get("retrieval.max_limit", 50))
        self.agent = str(cfg.get("identity.agent_id", "xiar"))

    def classify(self, query):
        answers = self.laya.decide(query[:self.max_chars], [{"type":"choice",
            "instructions":"Which retrieval strategy best answers the user's memory query?", "options":OPTIONS}])
        if answers is None:
            raise ValueError("Laya unavailable")
        probs = choice(answers[0], len(OPTIONS))
        i = max(range(len(probs)), key=probs.__getitem__)
        return dict(strategy=NAMES[i], confidence=probs[i], probabilities=probs)

    def search(self, business, query, limit, read_agent="", read_user=""):
        if not business:
            return {"error":"business_id is required"}
        if not query.strip():
            return {"hits":[],"reason":"empty_query"}
        limit = max(1, min(int(limit), self.max_limit))
        try:
            decision = self.classify(query)
        except (ValueError, TypeError, KeyError):
            self.audit.write(4,"unavailable_or_invalid")
            return {"hits":[],"reason":"Laya unavailable or invalid"}
        if decision["confidence"] < self.minimum:
            self.audit.write(4,"below_threshold",**decision)
            return {"hits":[],"reason":"below retrieval confidence threshold",**decision}
        strategies = decision["strategy"].split("+")
        result = {"business_id":business,"hits":[],"signals":[],"backend_methods":[],"ontology_reasons":[],**decision}
        seen=set()
        for strategy in strategies:
            if strategy == "brain":
                if self.agent == "xiar" and not read_agent and not read_user:
                    result["signals"] = self.brain.signals("memory retrieval: learned behaviour", "request: " + query[:400])
                continue
            recalled = self.memory.search(business, query, limit, ontology=(strategy=="ontology"),
                read_agent=read_agent, read_user=read_user)
            if "error" in recalled:
                self.audit.write(4,"memory_api_refused",**decision)
                return recalled
            method = recalled.get("method", "bm25")
            if method not in result["backend_methods"]: result["backend_methods"].append(method)
            ontology_reason = recalled.get("ontology_reason", "legacy_backend")
            if ontology_reason not in result["ontology_reasons"]: result["ontology_reasons"].append(ontology_reason)
            for field in ("agent_id","end_user_id"):
                if field in recalled:
                    result[field]=recalled[field]
            for hit in recalled.get("hits",[]):
                if hit.get("id") in seen:
                    continue
                seen.add(hit.get("id")); result["hits"].append(dict(hit,retrieval=strategy,method=method,ontology=bool(recalled.get("ontology",False))))
        result["hits"] = result["hits"][:limit]
        self.audit.write(4,"selected_strategy",hits=len(result["hits"]),signals=len(result["signals"]),backend_methods=result["backend_methods"],ontology_reasons=result["ontology_reasons"],**decision)
        return result