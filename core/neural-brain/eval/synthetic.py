# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Deterministic synthetic lesson set for the NeuralBrain eval harness.

Outcomes are assigned at random per action (seeded), so the brain cannot infer them from wording:
recall measures memory, not templates. Contains no secrets, money, contact or business facts.
"""
from __future__ import annotations

import random

TOOLS = {
    "terminal": ["restart the {x} service", "delete the {x} folder", "upgrade {x} packages", "rebuild {x} from source",
                 "clear the {x} cache", "kill the {x} process"],
    "git": ["force push the {x} branch", "rebase {x} onto main", "amend the last {x} commit", "squash the {x} history"],
    "files": ["overwrite the {x} config", "move the {x} folder", "rename {x} files", "archive old {x} logs"],
    "browser": ["log in to {x} again", "download the {x} report", "clear {x} cookies"],
    "messages": ["post the {x} update publicly", "reply to the {x} thread", "send the {x} summary without a draft"],
    "calendar": ["move the {x} meeting", "decline the {x} invite", "book the {x} slot"],
}
OBJECTS = ["hermes", "quickshell", "plymouth", "snapper", "limine", "chromium", "kiosk", "ontology", "memory",
           "desktop", "gateway", "scheduler", "orb", "metahuman", "tunnel", "trainer", "vault", "router",
           "voice", "screen", "dream", "reflex", "circadian", "backup"]
WHY = {
    "fail": ["it broke the boot last time", "this environment has no GPU for that", "it timed out twice before",
             "it lost the owner's work", "the permissions were wrong"],
    "rejected": ["the owner wants to be asked first", "the owner said never do this", "the owner prefers a draft first"],
    "approved": ["the owner approved this before", "the owner is fine with this"],
    "ok": ["it worked fine", "it finished cleanly"],
    "corrected": ["the owner said use the mirror instead", "the owner said snapshot first", "the owner said keep the fallback"],
}
PREFS = [("report format", "short bullet points"), ("time zone in replies", "America New York with a label"),
         ("project name", "always XIAR"), ("hermes folders", "never move them, mirror instead"),
         ("questions", "save them for the end"), ("build style", "no MVP, everything configurable"),
         ("old project", "read only"), ("snapshots", "take one before major system changes"),
         ("tokenizer", "open source, no weights"), ("desktop", "3D logo with the input bar"),
         ("colours", "pink and gold"), ("long runs", "do not stop to ask")]
CONTEXTS = ["session cli", "session desktop", "session gateway", "routine nightly", "subagent build", "subagent research"]


def build(seed: int = 7):
    rng = random.Random(seed)
    lessons = []
    for tool, verbs in TOOLS.items():
        for verb in verbs:
            for obj in rng.sample(OBJECTS, 4):
                outcome = rng.choice(["fail", "fail", "rejected", "approved", "ok", "corrected"])
                kind = {"fail": "mistake", "rejected": "rejection", "approved": "approval", "ok": "outcome",
                        "corrected": "correction"}[outcome]
                lessons.append({"kind": kind, "outcome": outcome, "context": rng.choice(CONTEXTS),
                                "action": f"{tool}: {verb.format(x=obj)}", "why": rng.choice(WHY[outcome]),
                                "source": "synthetic", "_tool": tool, "_verb": verb, "_obj": obj})
    for topic, value in PREFS:
        lessons.append({"kind": "preference", "outcome": "prefer", "context": "owner profile",
                        "action": f"preference: {topic}", "why": value, "source": "synthetic"})
    rng.shuffle(lessons)
    return lessons


def novel_actions(lessons, n: int = 40, seed: int = 11):
    """Actions never taught (unseen verb/object pairs or unseen verbs)."""
    rng = random.Random(seed)
    seen = {l["action"] for l in lessons}
    extra_verbs = ["print the {x} version", "list the {x} files", "open the {x} docs", "check {x} status"]
    pool = [f"{t}: {v.format(x=o)}" for t, vs in TOOLS.items() for v in vs + extra_verbs for o in OBJECTS]
    pool = [a for a in pool if a not in seen]
    rng.shuffle(pool)
    return pool[:n]


def split_days(lessons, days: int):
    per = (len(lessons) + days - 1) // days
    return [lessons[i * per:(i + 1) * per] for i in range(days)]


def paraphrase(l: dict) -> str:
    """Same action, different wording (for generalisation probe)."""
    a = l["action"]
    tool, rest = a.split(": ", 1)
    return f"{tool}: please {rest} now"
