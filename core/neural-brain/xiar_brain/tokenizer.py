# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Tokenizer wrapper. Loads ONLY a tokenizer.json (HF `tokenizers`); no model weights.

XIAR special tokens are appended after the base vocab, so the brain's vocab = base + specials.
"""
from __future__ import annotations

import pathlib
from typing import Iterable

from tokenizers import Tokenizer

from .config import BrainConfig


class BrainTokenizer:
    def __init__(self, path: pathlib.Path, specials: Iterable[str], name: str = ""):
        if not path.exists():
            raise FileNotFoundError(f"tokenizer.json not found at {path} (run `xiar-brain fetch-tokenizer`)")
        self.name = name
        self.tok = Tokenizer.from_file(str(path))
        self.base_vocab = self.tok.get_vocab_size()
        self.specials = list(specials)
        self.special_ids = {s: self.base_vocab + i for i, s in enumerate(self.specials)}
        self.vocab_size = self.base_vocab + len(self.specials)
        self.pad_id = self.special_ids["<pad>"]
        self.bos_id = self.special_ids["<bos>"]
        self.eos_id = self.special_ids["<eos>"]

    @classmethod
    def from_config(cls, cfg: BrainConfig) -> "BrainTokenizer":
        return cls(cfg.path("tokenizer.path"), cfg.get("tokenizer.specials"), cfg.get("tokenizer.name", ""))

    def sid(self, special: str) -> int:
        return self.special_ids[special]

    def encode_text(self, text: str) -> list[int]:
        return self.tok.encode(text, add_special_tokens=False).ids if text else []

    def decode(self, ids: Iterable[int]) -> str:
        base = [i for i in ids if i < self.base_vocab]
        return self.tok.decode(base).strip()
