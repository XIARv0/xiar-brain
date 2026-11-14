// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Search request model shared by every memory type, plus the text normalisation MIRIX applies before BM25.

use crate::filter_tags::Tags;

/// MIRIX search methods. `fuzzy_match` is kept for compatibility (it was already marked legacy upstream).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMethod {
    /// vector similarity on one embedded field
    Embedding,
    /// BM25 over the text fields (native sparse vector in Qdrant Edge)
    Bm25,
    /// case-insensitive substring containment
    StringMatch,
    /// typo-tolerant containment (legacy)
    FuzzyMatch,
    /// BM25 + embedding fused with RRF/DBSF/weighted (XIAR addition; MIRIX did this ad hoc for procedural memory)
    Hybrid,
}

impl SearchMethod {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "embedding" => Self::Embedding,
            "bm25" => Self::Bm25,
            "string_match" => Self::StringMatch,
            "fuzzy_match" => Self::FuzzyMatch,
            "hybrid" => Self::Hybrid,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fusion {
    Rrf,
    Dbsf,
    /// weights for [dense, sparse]
    Weighted(f32, f32),
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub user_id: String,
    pub query: String,
    /// precomputed query embedding (dimension must match the collection's)
    pub embedding: Option<Vec<f32>>,
    /// restrict to one field ("" = all searchable fields)
    pub field: String,
    pub method: SearchMethod,
    pub fusion: Fusion,
    pub limit: usize,
    pub filter_tags: Option<Tags>,
    /// `None` = no scope check; `Some(&[])` matches nothing
    pub scopes: Option<Vec<String>>,
    /// inclusive time window on `occurred_at` (unix ms), episodic memory only
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub similarity_threshold: Option<f32>,
}

impl SearchRequest {
    pub fn new(user_id: &str, query: &str) -> Self {
        Self {
            user_id: user_id.to_string(),
            query: query.to_string(),
            embedding: None,
            field: String::new(),
            method: SearchMethod::Bm25,
            fusion: Fusion::Rrf,
            limit: 50,
            filter_tags: None,
            scopes: None,
            start_ms: None,
            end_ms: None,
            similarity_threshold: None,
        }
    }
}

/// MIRIX `_clean_text_for_search`: punctuation -> spaces, lowercase, collapse whitespace.
pub fn clean_text(text: &str) -> String {
    let replaced: String = text.chars().map(|c| if c.is_ascii_punctuation() { ' ' } else { c }).collect();
    replaced.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// MIRIX `_preprocess_text_for_bm25`: cleaned tokens longer than one character.
pub fn bm25_tokens(text: &str) -> Vec<String> {
    clean_text(text).split(' ').filter(|t| t.chars().count() > 1).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning_matches_mirix_behaviour() {
        assert_eq!(clean_text("  Hello,   WORLD!! It's me.  "), "hello world it s me");
        assert_eq!(clean_text(""), "");
        assert_eq!(bm25_tokens("A big, fast-car; x"), vec!["big", "fast", "car"], "single-char tokens dropped");
        assert_eq!(bm25_tokens("Ünïcode Wörds ok"), vec!["ünïcode", "wörds", "ok"]);
    }

    #[test]
    fn method_names_round_trip() {
        for (s, m) in [("embedding", SearchMethod::Embedding), ("bm25", SearchMethod::Bm25), ("string_match", SearchMethod::StringMatch), ("fuzzy_match", SearchMethod::FuzzyMatch), ("hybrid", SearchMethod::Hybrid)] {
            assert_eq!(SearchMethod::parse(s), Some(m));
        }
        assert_eq!(SearchMethod::parse("regex"), None);
    }

    #[test]
    fn request_defaults() {
        let r = SearchRequest::new("u1", "q");
        assert_eq!((r.limit, r.method, r.scopes.is_none()), (50, SearchMethod::Bm25, true));
    }
}
