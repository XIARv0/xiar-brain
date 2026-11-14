// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Optional query expansion for ontology-aware retrieval.
//!
//! The expander is injected by the host (GUI/agents) when a promoted ontology is available.
//! Money and Knowledge Vault are never expanded through this path.

/// Widens a search string with related terms (e.g. ontology class names).
pub trait QueryExpander: Send + Sync {
    fn expand(&self, query: &str) -> Vec<String>;
}

/// No-op expander used when ontology is off.
pub struct NullExpander;

impl QueryExpander for NullExpander {
    fn expand(&self, _query: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Join original query with expanded terms (deduped, lowercased), space-separated.
pub fn merge_query(query: &str, expanded: &[String]) -> String {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for t in query.split_whitespace().chain(expanded.iter().flat_map(|s| s.split_whitespace())) {
        let l: String = t
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .flat_map(|c| c.to_lowercase())
            .collect();
        if l.chars().count() > 1 && seen.insert(l.clone()) {
            out.push(l);
        }
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_dedupes_and_lowercases() {
        assert_eq!(merge_query("Which Vendors?", &["Vendor".into(), "vendors".into()]), "which vendors vendor");
        assert_eq!(merge_query("x", &[]), "");
    }
}
