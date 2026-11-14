// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Data shared across the ontology agents and the registry.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Namespace of every generated class and property.
pub const NS: &str = "http://xiar.ai/ontology#";
pub const ONTOLOGY_IRI: &str = "http://xiar.ai/ontology";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateClass {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub source_document: String,
    pub confidence: f64,
    #[serde(default = "one")]
    pub occurrences: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateRelation {
    pub subject_class: String,
    pub predicate: String,
    pub object_class: String,
    #[serde(default)]
    pub description: String,
    pub source_document: String,
    pub confidence: f64,
    #[serde(default = "one")]
    pub occurrences: u32,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OntologyCandidate {
    #[serde(default)]
    pub classes: Vec<CandidateClass>,
    #[serde(default)]
    pub relations: Vec<CandidateRelation>,
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineerResponse {
    pub new_version: String,
    pub diff_summary: String,
    pub classes_added: usize,
    pub relations_added: usize,
    pub conflicts_resolved: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Draft,
    Validated,
    Promoted,
    Deprecated,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::Validated => "validated",
            Status::Promoted => "promoted",
            Status::Deprecated => "deprecated",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "draft" => Status::Draft,
            "validated" => Status::Validated,
            "promoted" => Status::Promoted,
            "deprecated" => Status::Deprecated,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionInfo {
    pub version: String,
    pub timestamp_ms: u64,
    pub promoted_by: String,
    pub migration_required: bool,
    pub diff_summary: Option<String>,
    pub status: Status,
    /// SHA-256 of the stored ontology text
    pub file_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub quality_score: f64,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub suggestions: Vec<String>,
    pub class_count: usize,
    pub relation_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsistencyResult {
    pub is_consistent: bool,
    pub checked_at_ms: u64,
    pub reasoner: String,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub execution_time_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionResponse {
    pub version: String,
    pub previous_status: Status,
    pub new_status: Status,
    pub quality_score: f64,
    pub promoted: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReIngestAction {
    NoAction,
    PartialReingest,
    FullReingest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReIngestCheck {
    pub action: ReIngestAction,
    pub reason: String,
    pub affected_documents: Vec<String>,
    pub document_ontology_version: Option<String>,
}
