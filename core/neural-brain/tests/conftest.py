# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
import pathlib, shutil, sys
import pytest

PKG = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PKG))
REPO = PKG.parents[1]

from xiar_brain.config import load_config  # noqa: E402


@pytest.fixture
def cfg(tmp_path):
    """Tiny config in a temp root; tokenizer from the repo."""
    c = load_config(REPO / "config" / "neural_brain.toml", root=tmp_path)
    tok = tmp_path / "tok" / "tokenizer.json"
    tok.parent.mkdir()
    shutil.copy(REPO / c.get("tokenizer.path"), tok)
    return c.with_overrides({
        "tokenizer.path": str(tok), "train.device": "cpu", "model.d_model": 64, "model.n_layers": 2,
        "model.n_heads": 2, "model.memory_layers": [1], "memory.n_keys": 16, "memory.k_dim": 32,
        "memory.topk": 8, "memory.heads": 2, "memory.grow_step": 4, "memory.max_n_keys": 32,
        "train.epochs_per_window": 3, "smf.bootstrap_lessons": 4, "smf.top_t": 50, "train.batch_size": 8,
    })


@pytest.fixture
def keys(cfg):
    from xiar_brain.sealed_store import generate_keys
    generate_keys(cfg.path("paths.sealed_public_key"), cfg.path("paths.sealed_private_key"))
    return cfg
