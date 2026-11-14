# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
import torch
from xiar_brain.brain import Brain
from xiar_brain.pkm import ProductKeyMemory

LESSONS = [{"kind": "mistake", "action": f"terminal: restart service {i}", "outcome": "fail" if i % 2 else "ok",
            "why": "it broke before", "context": "session cli"} for i in range(8)]


def test_newborn_random_init_no_pretrained(cfg):
    a = Brain(cfg).model.state_dict()
    b = Brain(cfg.with_overrides({"model.seed": 1})).model.state_dict()
    assert not torch.equal(a["tok.weight"], b["tok.weight"]), "weights come from the seed, not a pretrained file"
    assert torch.equal(a["tok.weight"], Brain(cfg).model.state_dict()["tok.weight"]), "same seed -> same newborn"


def test_growth_keeps_old_slots():
    torch.manual_seed(0)
    m = ProductKeyMemory(16, 8, 2, 8, 4)
    old = m.values.weight.data.clone().view(8, 8, 16)
    m.grow(4)
    new = m.values.weight.data.view(12, 12, 16)
    assert m.n_keys == 12 and torch.equal(new[:8, :8], old) and new[8:].abs().sum() == 0


def test_sparse_memory_finetuning_only_touches_top_t_rows(cfg):
    c = cfg.with_overrides({"smf.bootstrap_lessons": 0, "smf.top_t": 5, "train.epochs_per_window": 1,
                            "memory.grow_threshold": 2.0})
    b = Brain(c)
    before_vals = [m.values.weight.detach().clone() for m in b.model.memories()]
    before_other = b.model.tok.weight.detach().clone()
    b.train_window(LESSONS[:4], LESSONS[4:])
    steps = 1  # one batch of 4 new (+replay) per epoch
    for bv, m in zip(before_vals, b.model.memories()):
        changed = (m.values.weight.detach() != bv).any(dim=1).sum().item()
        assert 0 < changed <= 5 * steps
    assert torch.equal(before_other, b.model.tok.weight.detach()), "non-memory params frozen after bootstrap"


def test_checkpoint_roundtrip_and_signals(cfg, tmp_path):
    b = Brain(cfg.with_overrides({"train.epochs_per_window": 2}))
    b.train_window(LESSONS, [])
    p = tmp_path / "ck.pt"; b.save(p)
    b2 = Brain.load(cfg, p)
    s1, s2 = b.signals("session cli", LESSONS[0]["action"]), b2.signals("session cli", LESSONS[0]["action"])
    assert s1["probs"] == s2["probs"]
    assert set(s1) >= {"label", "text", "confidence", "familiarity"}
