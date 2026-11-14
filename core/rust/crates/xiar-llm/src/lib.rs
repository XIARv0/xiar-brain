// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! xiar-llm — the engine contract. Everything that generates text or embeddings sits behind these traits, so the core
//! never depends on a model runtime: mistral.rs, llama.cpp, OpenVINO and cloud providers are plug-ins (Phase 6).
//!
//! * [`LlmEngine`]: one chat completion with tool calling. Synchronous: callers run agents on threads.
//! * [`EmbedEngine`]: text -> vectors.
//! * [`Router`]: agent -> profile -> engine, from `xiar_core::config` (default everything to one profile; per-agent
//!   overrides; unknown profiles are errors).
//! * [`testing`]: a scripted engine and a deterministic hashing embedder for tests (no model needed).

pub mod decision;
pub mod laya;
pub mod router;
pub mod testing;
pub mod types;

pub use laya::Laya;
pub use router::Router;
pub use types::*;
