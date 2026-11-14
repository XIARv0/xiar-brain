// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The ontology service as a library (port of the legacy Python/FastAPI service, without containers or Ollama).
//! Pieces: candidate extraction (`scout`), merging into a stable ontology (`engineer`), review and scoring (`critic`),
//! logical checks with the own OWL reasoner (`consistency`), the versioned `registry`, and the `service` that ties them
//! together (promotion with a quality gate, re-ingestion checks, export, query expansion).

pub mod text;
pub mod engineer;
pub mod models;

pub mod consistency;
pub mod critic;
pub mod registry;
pub mod scout;
pub mod service;

/// The prompts the agents ship with (editable copies live in the user's prompt folder).
pub const BUNDLED: &[(&str, &str)] = &[
    ("ontology/scout_classes", include_str!("../../../prompts/ontology/scout_classes.md")),
    ("ontology/scout_relations", include_str!("../../../prompts/ontology/scout_relations.md")),
    ("ontology/critic_review", include_str!("../../../prompts/ontology/critic_review.md")),
];
