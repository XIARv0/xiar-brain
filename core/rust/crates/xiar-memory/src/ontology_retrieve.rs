// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Ontology-aware semantic retrieval: BM25 on (optionally expanded) query plus
//! `list_with_any` over `entity_types` for expanded class terms, fused by RRF.
//!
//! This is the unfinished MigrationPlan experiment: class-level questions
//! ("which vendors?") need typed semantic memories + expansion to beat plain BM25.

use crate::manager::{MemoryManager, Result, Scored};
use crate::query_expand::{merge_query, QueryExpander};
use crate::search::{SearchMethod, SearchRequest};
use crate::types::SemanticItem;
use std::collections::BTreeMap;
use std::sync::Arc;

const RRF_K: f32 = 60.0;

/// Run semantic search with ontology off (`expander = None`) or on.
pub fn search_semantic_ontology(
    semantic: &MemoryManager<SemanticItem>,
    user: &str,
    query: &str,
    limit: usize,
    scopes: Option<&[String]>,
    expander: Option<&dyn QueryExpander>,
) -> Result<Vec<Scored<SemanticItem>>> {
    if limit == 0 || query.trim().is_empty() {
        return Ok(vec![]);
    }
    let expanded = expander.map(|e| e.expand(query)).unwrap_or_default();
    let q = if expanded.is_empty() {
        query.to_string()
    } else {
        merge_query(query, &expanded)
    };

    let mut req = SearchRequest::new(user, &q);
    req.method = SearchMethod::Bm25;
    req.limit = limit.saturating_mul(3).max(limit);
    if let Some(s) = scopes {
        req.scopes = Some(s.to_vec());
    }
    let bm25 = semantic.search(&req)?;

    let class_hits = if expanded.is_empty() {
        Vec::new()
    } else {
        semantic.list_with_any(user, "entity_types", &expanded, req.limit, scopes, None)?
    };

    Ok(fuse_rrf(bm25, class_hits, limit))
}

fn fuse_rrf(bm25: Vec<Scored<SemanticItem>>, typed: Vec<SemanticItem>, limit: usize) -> Vec<Scored<SemanticItem>> {
    let mut scores: BTreeMap<String, f32> = BTreeMap::new();
    let mut items: BTreeMap<String, SemanticItem> = BTreeMap::new();
    for (rank, s) in bm25.into_iter().enumerate() {
        let id = s.item.common.id.clone();
        *scores.entry(id.clone()).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        items.insert(id, s.item);
    }
    for (rank, item) in typed.into_iter().enumerate() {
        let id = item.common.id.clone();
        *scores.entry(id.clone()).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        items.entry(id).or_insert(item);
    }
    let mut ranked: Vec<_> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked
        .into_iter()
        .take(limit)
        .filter_map(|(id, score)| items.remove(&id).map(|item| Scored { item, score }))
        .collect()
}

/// Thin adapter: host closes over OntologyService::expand_query.
pub struct FnExpander(pub Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>);

impl QueryExpander for FnExpander {
    fn expand(&self, query: &str) -> Vec<String> {
        (self.0)(query)
    }
}
