// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! xiar-memory — domain model of the six MIRIX memory layers (+ raw memory, skill experience, core blocks).
//!
//! Derived from MIRIX `schemas/*` and `orm/*` (Apache-2.0; modified: Rust port, SQLAlchemy/pgvector replaced by the
//! store traits in `xiar-store`, organization collapsed away, embeddings kept at their real dimension instead of
//! being zero-padded to a fixed size).
//!
//! * [`ids`]: prefixed ids (`ep_mem-<uuid>`), UUIDv7 for raw memories.
//! * [`types`]: the memory item structs and their updates.
//! * [`filter_tags`]: MIRIX's `filter_tags` query semantics (`$contains`, `$exists`, `$in`, scope authorization).
//! * [`search`]: the search request model shared by every memory type, and text normalisation for BM25.

pub mod core;
pub mod experience;
pub mod filter_tags;
pub mod hermes_organise;
pub mod hermes_sync;
pub mod hub;
pub mod ids;
pub mod manager;
pub mod raw;
pub mod registry;
mod records;
pub mod search;
pub mod sources;
pub mod types;
pub mod business_ns;
pub mod governance;
pub mod ontology_retrieve;
pub mod query_expand;
pub mod tenant;

#[cfg(test)]
mod hub_tests;
#[cfg(test)]
mod manager_tests;

pub use types::*;
