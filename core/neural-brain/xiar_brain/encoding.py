# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Lesson <-> token sequences.

Training sequence:  <bos> <ctx> context <act> action <out> <LABEL> <why> why <eos>
Query prompt:       <bos> <ctx> context <act> action <out>      -> next token over labels, then why
"""
from __future__ import annotations

from dataclasses import dataclass

from .tokenizer import BrainTokenizer

LABEL_TOKENS = {"ok": "<ok>", "fail": "<fail>", "rejected": "<rejected>", "approved": "<approved>",
                "prefer": "<prefer>", "corrected": "<corrected>"}


@dataclass
class Encoded:
    ids: list[int]
    label_pos: int          # index in ids of the label token
    why_start: int          # first why token index
    action_span: tuple[int, int]


def _clip(ids: list[int], n: int) -> list[int]:
    return ids[: max(0, n)]


def encode_lesson(tok: BrainTokenizer, lesson: dict, max_len: int) -> Encoded:
    ctx = tok.encode_text(lesson.get("context", ""))
    act = tok.encode_text(lesson.get("action", ""))
    why = tok.encode_text(lesson.get("why", ""))
    budget = max_len - 7
    act = _clip(act, budget // 2)
    ctx = _clip(ctx, (budget - len(act)) // 2)
    why = _clip(why, budget - len(act) - len(ctx))
    ids = [tok.bos_id, tok.sid("<ctx>"), *ctx, tok.sid("<act>")]
    a0 = len(ids)
    ids += act
    a1 = len(ids)
    ids += [tok.sid("<out>")]
    label_pos = len(ids)
    ids += [tok.sid(LABEL_TOKENS[lesson["outcome"]]), tok.sid("<why>")]
    why_start = len(ids)
    ids += why + [tok.eos_id]
    return Encoded(ids, label_pos, why_start, (a0, a1))


def encode_query(tok: BrainTokenizer, context: str, action: str, max_len: int) -> Encoded:
    enc = encode_lesson(tok, {"context": context, "action": action, "outcome": "ok", "why": ""}, max_len)
    ids = enc.ids[: enc.label_pos]
    return Encoded(ids, enc.label_pos, enc.why_start, enc.action_span)


def label_ids(tok: BrainTokenizer) -> dict[str, int]:
    return {k: tok.sid(v) for k, v in LABEL_TOKENS.items()}
