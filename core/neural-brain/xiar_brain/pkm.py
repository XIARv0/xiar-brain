# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Product-key memory layer (Lample et al. 2019; Meta "Memory Layers at Scale", arXiv 2412.09764,
reference github.com/facebookresearch/memory), written from scratch for XIAR.

slots = n_keys^2. Each head scores the two query halves against two sub-key tables, takes top-k of
each, combines the k*k candidates and keeps the global top-k. Output = softmax-weighted sum of value
rows (optionally SiLU-gated). The layer records which slots were accessed so sparse memory
finetuning can rank them by TF-IDF, and supports growing n_keys without losing old slots.
"""
from __future__ import annotations

import math

import torch
import torch.nn as nn
import torch.nn.functional as F


class ProductKeyMemory(nn.Module):
    def __init__(self, d_model: int, n_keys: int, heads: int, k_dim: int, topk: int,
                 gated: bool = True, value_init_std: float = 0.02):
        super().__init__()
        assert k_dim % 2 == 0
        self.d_model, self.n_keys, self.heads, self.k_dim, self.topk = d_model, n_keys, heads, k_dim, topk
        self.half = k_dim // 2
        self.query = nn.Linear(d_model, heads * k_dim, bias=False)
        self.query_norm = nn.LayerNorm(k_dim)
        self.keys = nn.Parameter(torch.randn(heads, 2, n_keys, self.half) / math.sqrt(self.half))
        self.values = nn.Embedding(n_keys * n_keys, d_model)
        nn.init.normal_(self.values.weight, std=value_init_std)
        self.gate = nn.Linear(d_model, d_model, bias=False) if gated else None
        self.out = nn.Linear(d_model, d_model, bias=False)
        self.value_init_std = value_init_std
        self.last_indices: torch.Tensor | None = None   # (N, heads*topk) flat slot ids of last forward
        self.record = False

    @property
    def n_slots(self) -> int:
        return self.n_keys * self.n_keys

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        B, T, D = x.shape
        q = self.query(x).view(B * T, self.heads, self.k_dim)
        q = self.query_norm(q)
        q1, q2 = q[..., : self.half], q[..., self.half:]
        k = min(self.topk, self.n_keys)
        s1 = torch.einsum("nhd,hkd->nhk", q1, self.keys[:, 0])
        s2 = torch.einsum("nhd,hkd->nhk", q2, self.keys[:, 1])
        v1, i1 = s1.topk(k, dim=-1)
        v2, i2 = s2.topk(k, dim=-1)
        cand = (v1.unsqueeze(-1) + v2.unsqueeze(-2)).view(B * T, self.heads, k * k)
        cidx = (i1.unsqueeze(-1) * self.n_keys + i2.unsqueeze(-2)).view(B * T, self.heads, k * k)
        top_v, top_pos = cand.topk(k, dim=-1)
        idx = cidx.gather(-1, top_pos)                              # (N, H, k)
        w = F.softmax(top_v, dim=-1)
        vals = self.values(idx)                                     # (N, H, k, D)
        o = (w.unsqueeze(-1) * vals).sum(dim=(1, 2))                # (N, D)
        if self.record:
            self.last_indices = idx.reshape(B * T, -1).detach()
        o = o.view(B, T, D)
        if self.gate is not None:
            o = o * F.silu(self.gate(x))
        return self.out(o)

    @torch.no_grad()
    def grow(self, add: int, key_scale: float = 0.5) -> None:
        """n_keys -> n_keys+add. Old slot (i,j) keeps its sub-keys and value at new flat id i*K'+j;
        new slots start with zero values so behaviour is preserved until they are trained."""
        K, K2 = self.n_keys, self.n_keys + add
        dev = self.keys.device
        new_keys = torch.randn(self.heads, 2, add, self.half, device=dev) / math.sqrt(self.half) * key_scale
        self.keys = nn.Parameter(torch.cat([self.keys.data, new_keys], dim=2))
        old = self.values.weight.data.view(K, K, self.d_model)
        nv = torch.zeros(K2, K2, self.d_model, device=dev, dtype=old.dtype)
        nv[:K, :K] = old
        self.values = nn.Embedding(K2 * K2, self.d_model).to(dev)
        self.values.weight.data.copy_(nv.view(K2 * K2, self.d_model))
        self.n_keys = K2

    @staticmethod
    def remap_old_ids(ids: torch.Tensor, old_k: int, new_k: int) -> torch.Tensor:
        return (ids // old_k) * new_k + (ids % old_k)
