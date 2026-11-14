// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Per-business memory namespaces (`business:<slug>`) with a shared Core scope.
//!
//! Semantic / Episodic / Procedural / Resource carry `filter_tags.scope = business:<slug>`.
//! Core blocks for the user stay on the shared `core` scope (not per-business).
//! Cross-namespace reads are denied unless the caller lists the scope in `scopes`.

use crate::filter_tags::{self, Tags};
use serde_json::json;

pub const SCOPE_CORE: &str = "core";
pub const BUSINESS_PREFIX: &str = "business:";

/// `business:<slug>` after normalizing slug (lowercase, `[a-z0-9_-]` only).
pub fn business_scope(slug: &str) -> Result<String, String> {
    let s = slug.trim().to_lowercase();
    if s.is_empty() {
        return Err("business slug is empty".into());
    }
    if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("invalid business slug {slug:?}"));
    }
    Ok(format!("{BUSINESS_PREFIX}{s}"))
}

pub fn is_business_scope(scope: &str) -> bool {
    scope.starts_with(BUSINESS_PREFIX) && scope.len() > BUSINESS_PREFIX.len()
}

/// Filter tags for an item owned by one business namespace.
pub fn business_tags(slug: &str) -> Result<Tags, String> {
    let scope = business_scope(slug)?;
    Ok(json!({ "scope": scope, "tags": [scope.clone(), "business"] })
        .as_object()
        .cloned()
        .expect("object"))
}

/// Shared Core tags (user-level, not per business).
pub fn core_tags() -> Tags {
    json!({ "scope": SCOPE_CORE, "tags": [SCOPE_CORE] })
        .as_object()
        .cloned()
        .expect("object")
}

/// Does `authorized` allow reading an item with `stored` tags?
/// `None` = no scope check (legacy / admin). `Some([])` = authorized for nothing.
pub fn may_read(stored: Option<&Tags>, authorized: Option<&[String]>) -> Result<bool, filter_tags::FilterError> {
    filter_tags::matches(stored, None, authorized)
}

/// Cross-business read is denied when the caller only holds one business scope.
pub fn deny_cross_business(caller_scopes: &[String], item_scope: &str) -> bool {
    if !is_business_scope(item_scope) {
        return false;
    }
    !caller_scopes.iter().any(|s| s == item_scope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn slug_normalizes_and_rejects_junk() {
        assert_eq!(business_scope(" tenant-a ").unwrap(), "business:tenant-a");
        assert!(business_scope("").is_err());
        assert!(business_scope("a/b").is_err());
    }

    #[test]
    fn isolation_denies_other_business() {
        let tenant_a = business_tags("tenant-a").unwrap();
        let tenant_b = business_tags("tenant-b").unwrap();
        let only_tenant_a = vec!["business:tenant-a".to_string()];
        assert!(may_read(Some(&tenant_a), Some(&only_tenant_a)).unwrap());
        assert!(!may_read(Some(&tenant_b), Some(&only_tenant_a)).unwrap());
        assert!(may_read(Some(&tenant_b), None).unwrap(), "None = no scope check");
        assert!(!may_read(Some(&tenant_a), Some(&[])).unwrap());
    }

    #[test]
    fn core_is_shared_not_business() {
        assert!(!is_business_scope(SCOPE_CORE));
        let c = core_tags();
        assert_eq!(c.get("scope").and_then(|v| v.as_str()), Some("core"));
        let _ = json!(null); // keep serde_json used in tests via tags
    }

    #[test]
    fn deny_cross_business_helper() {
        assert!(deny_cross_business(&["business:a".into()], "business:b"));
        assert!(!deny_cross_business(&["business:a".into(), "business:b".into()], "business:b"));
        assert!(!deny_cross_business(&["business:a".into()], "core"));
    }
}
