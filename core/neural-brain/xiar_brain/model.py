# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Small causal transformer with product-key memory layers. Random init only (newborn)."""
from __future__ import annotations

import torch
import torch.nn as nn
import torch.nn.functional as F

from .config import BrainConfig
from .pkm import ProductKeyMemory


class Block(nn.Module):
    def __init__(self, d: int, heads: int, ffn_mult: int, dropout: float, memory: ProductKeyMemory | None):
        super().__init__()
        self.ln1 = nn.LayerNorm(d)
        self.attn = nn.MultiheadAttention(d, heads, dropout=dropout, batch_first=True)
        self.ln2 = nn.LayerNorm(d)
        self.memory = memory
        self.ffn = None if memory is not None else nn.Sequential(
            nn.Linear(d, d * ffn_mult), nn.GELU(), nn.Linear(d * ffn_mult, d))
        self.drop = nn.Dropout(dropout)

    def forward(self, x, mask, pad_mask):
        h = self.ln1(x)
        a, _ = self.attn(h, h, h, attn_mask=mask, key_padding_mask=pad_mask, need_weights=False)
        x = x + self.drop(a)
        h = self.ln2(x)
        x = x + self.drop(self.memory(h) if self.memory is not None else self.ffn(h))
        return x


class BrainModel(nn.Module):
    def __init__(self, vocab_size: int, pad_id: int, *, d_model: int, n_layers: int, n_heads: int,
                 ffn_mult: int, max_len: int, dropout: float, memory_layers: list[int], tie_embeddings: bool,
                 mem: dict):
        super().__init__()
        self.hparams = dict(vocab_size=vocab_size, pad_id=pad_id, d_model=d_model, n_layers=n_layers,
                            n_heads=n_heads, ffn_mult=ffn_mult, max_len=max_len, dropout=dropout,
                            memory_layers=list(memory_layers), tie_embeddings=tie_embeddings, mem=dict(mem))
        self.pad_id = pad_id
        self.max_len = max_len
        self.tok = nn.Embedding(vocab_size, d_model)
        self.pos = nn.Embedding(max_len, d_model)
        self.blocks = nn.ModuleList()
        for i in range(n_layers):
            m = ProductKeyMemory(d_model, mem["n_keys"], mem["heads"], mem["k_dim"], mem["topk"],
                                 mem.get("gated", True), mem.get("value_init_std", 0.02)) if i in memory_layers else None
            self.blocks.append(Block(d_model, n_heads, ffn_mult, dropout, m))
        self.ln = nn.LayerNorm(d_model)
        self.head = nn.Linear(d_model, vocab_size, bias=False)
        if tie_embeddings:
            self.head.weight = self.tok.weight
        nn.init.normal_(self.tok.weight, std=0.02)

    @classmethod
    def from_config(cls, cfg: BrainConfig, vocab_size: int, pad_id: int) -> "BrainModel":
        torch.manual_seed(int(cfg.get("model.seed", 0)))
        m = cfg.section("model")
        return cls(vocab_size, pad_id, d_model=m["d_model"], n_layers=m["n_layers"], n_heads=m["n_heads"],
                   ffn_mult=m["ffn_mult"], max_len=m["max_len"], dropout=m["dropout"],
                   memory_layers=m["memory_layers"], tie_embeddings=m.get("tie_embeddings", True),
                   mem=cfg.section("memory"))

    def memories(self) -> list[ProductKeyMemory]:
        return [b.memory for b in self.blocks if b.memory is not None]

    def forward(self, ids: torch.Tensor) -> torch.Tensor:
        B, T = ids.shape
        pos = torch.arange(T, device=ids.device)
        x = self.tok(ids) + self.pos(pos)[None]
        mask = torch.triu(torch.ones(T, T, device=ids.device, dtype=torch.bool), 1)
        pad = ids.eq(self.pad_id)
        for b in self.blocks:
            x = b(x, mask, pad)
        return self.head(self.ln(x))

    def n_params(self) -> int:
        return sum(p.numel() for p in self.parameters())


def seq_logprobs(model: BrainModel, ids: torch.Tensor) -> torch.Tensor:
    """log p(ids[t] | ids[<t]) for t>=1, shape (B, T-1)."""
    logits = model(ids[:, :-1])
    return F.log_softmax(logits.float(), -1).gather(-1, ids[:, 1:].unsqueeze(-1)).squeeze(-1)
