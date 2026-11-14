// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Raw memory operations: update (append/replace context, merge/replace tags), TTL purge and paged listing.
//! Ported from MIRIX `raw_memory_manager` and `jobs/cleanup_raw_memories.py`.

use crate::filter_tags::Tags;
use crate::manager::{MemError, MemoryManager, Result};
use crate::types::{now_ms, LastModify, Ms, RawMemory, RAW_MEMORY_TTL_DAYS};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMode {
    Replace,
    /// joined with a blank line, as MIRIX does
    Append,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagsMode {
    Replace,
    Merge,
}

/// Who is changing a raw memory. A client may only change memories of its own write scope; a client WITHOUT a write scope is
/// read-only and may change nothing (the pinned upstream refuses it with "Access denied ... actor has write_scope 'None'").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Writer {
    /// the system itself (retention, restore): no scope check
    #[default]
    System,
    Client { write_scope: Option<String> },
}

#[derive(Debug, Clone)]
pub struct RawUpdate {
    pub context: Option<String>,
    pub context_mode: ContextMode,
    pub tags: Option<Tags>,
    pub tags_mode: TagsMode,
    /// Who is writing. For a client the memory's scope must equal its write scope and the update may not change the scope.
    pub writer: Writer,
}

impl Default for RawUpdate {
    fn default() -> Self {
        Self { context: None, context_mode: ContextMode::Replace, tags: None, tags_mode: TagsMode::Replace, writer: Writer::System }
    }
}

fn scope_of(tags: &Option<Tags>) -> Option<String> {
    tags.as_ref().and_then(|t| t.get("scope")).and_then(|v| v.as_str()).map(str::to_string)
}

/// Update a raw memory. `new_embedding` (for the `context` vector) must be supplied when the context changes if the
/// collection is embedded; otherwise the old vector is kept.
pub fn update_raw(mgr: &MemoryManager<RawMemory>, user: &str, id: &str, upd: &RawUpdate, new_embedding: Option<Vec<f32>>) -> Result<RawMemory> {
    // pre-check on the current state so access errors are reported distinctly from "not found"
    let current = mgr.get(user, id, None)?.ok_or_else(|| MemError::NotFound(format!("raw memory {id}")))?;
    if let Writer::Client { write_scope } = &upd.writer {
        let mem_scope = scope_of(&current.filter_tags);
        // the words are upstream's, which prints a missing scope as None
        if mem_scope != *write_scope {
            return Err(MemError::Invalid(format!("Access denied: memory {id} has scope '{}', actor has write_scope '{}'", mem_scope.as_deref().unwrap_or("None"), write_scope.as_deref().unwrap_or("None"))));
        }
        if let Some(t) = &upd.tags {
            if let Some(new_scope) = t.get("scope") {
                if new_scope.as_str() != write_scope.as_deref() {
                    return Err(MemError::Invalid("Cannot change memory scope - scope must match actor.write_scope".into()));
                }
            }
        }
    }
    let emb: BTreeMap<String, Vec<f32>> = match (&upd.context, new_embedding) {
        (Some(_), Some(v)) => BTreeMap::from([("context".to_string(), v)]),
        _ => BTreeMap::new(),
    };
    let upd = upd.clone();
    let (item, _) = mgr
        .update(user, id, emb, move |m| {
            if let Some(c) = &upd.context {
                m.context = match upd.context_mode {
                    ContextMode::Append => format!("{}\n\n{}", m.context, c),
                    ContextMode::Replace => c.clone(),
                };
            }
            if let Some(t) = &upd.tags {
                match upd.tags_mode {
                    TagsMode::Merge => {
                        let mut merged = m.filter_tags.clone().unwrap_or_default();
                        merged.extend(t.clone());
                        m.filter_tags = Some(merged);
                    }
                    TagsMode::Replace => {
                        let preserved = scope_of(&m.filter_tags);
                        let mut new_tags = t.clone();
                        if let Some(s) = preserved {
                            new_tags.insert("scope".into(), serde_json::Value::String(s));
                        }
                        m.filter_tags = Some(new_tags);
                    }
                }
            }
            m.updated_at_ms = now_ms();
            m.last_modify = LastModify::now("updated");
            vec!["raw"]
        })?
        .ok_or_else(|| MemError::NotFound(format!("raw memory {id}")))?;
    Ok(item)
}

/// Delete raw memories not updated for more than 14 days (strictly older). Returns how many were removed. Never
/// touches the audit ledger.
pub fn purge_expired(mgr: &MemoryManager<RawMemory>, now: Ms) -> Result<usize> {
    mgr.purge_updated_before(now - RAW_MEMORY_TTL_DAYS * 24 * 3600 * 1000)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    UpdatedAt,
    CreatedAt,
    OccurredAt,
}

#[derive(Debug, Clone, Default)]
pub struct TimeRange {
    pub created_gte: Option<Ms>,
    pub created_lte: Option<Ms>,
    pub occurred_gte: Option<Ms>,
    pub occurred_lte: Option<Ms>,
    pub updated_gte: Option<Ms>,
    pub updated_lte: Option<Ms>,
}

#[derive(Debug, Clone)]
pub struct RawQuery {
    pub filter_tags: Option<Tags>,
    pub scopes: Option<Vec<String>>,
    pub sort: SortKey,
    pub descending: bool,
    pub time_range: TimeRange,
    pub cursor: Option<String>,
    /// 1..=100
    pub limit: usize,
}

impl Default for RawQuery {
    fn default() -> Self {
        Self { filter_tags: None, scopes: None, sort: SortKey::UpdatedAt, descending: true, time_range: TimeRange::default(), cursor: None, limit: 10 }
    }
}

#[derive(Debug, Clone)]
pub struct RawPage {
    pub items: Vec<RawMemory>,
    pub cursor: Option<String>,
}

/// Parse MIRIX sort strings: `updated_at`, `-updated_at`, `created_at`, `-created_at`, `occurred_at`, `-occurred_at`.
pub fn parse_sort(s: &str) -> Result<(SortKey, bool)> {
    let (desc, name) = match s.strip_prefix('-') {
        Some(n) => (true, n),
        None => (false, s),
    };
    let key = match name {
        "updated_at" => SortKey::UpdatedAt,
        "created_at" => SortKey::CreatedAt,
        "occurred_at" => SortKey::OccurredAt,
        _ => return Err(MemError::Invalid(format!("invalid sort {s:?}"))),
    };
    Ok((key, desc))
}

fn in_range(v: Ms, gte: Option<Ms>, lte: Option<Ms>) -> bool {
    gte.is_none_or(|g| v >= g) && lte.is_none_or(|l| v <= l)
}

/// Paged listing of a user's raw memories (cursor = opaque position token).
pub fn list_raw(mgr: &MemoryManager<RawMemory>, user: &str, q: &RawQuery) -> Result<RawPage> {
    if !(1..=100).contains(&q.limit) {
        return Err(MemError::Invalid("limit must be between 1 and 100".into()));
    }
    let offset: usize = match &q.cursor {
        None => 0,
        Some(c) => c.strip_prefix("o:").and_then(|n| n.parse().ok()).ok_or_else(|| MemError::Invalid("invalid cursor".into()))?,
    };
    let t = &q.time_range;
    let mut all: Vec<RawMemory> = mgr
        .list_recent(user, 0, q.scopes.as_deref(), q.filter_tags.as_ref())?
        .into_iter()
        .filter(|m| in_range(m.created_at_ms, t.created_gte, t.created_lte) && in_range(m.occurred_at_ms, t.occurred_gte, t.occurred_lte) && in_range(m.updated_at_ms, t.updated_gte, t.updated_lte))
        .collect();
    let key = |m: &RawMemory| match q.sort {
        SortKey::UpdatedAt => m.updated_at_ms,
        SortKey::CreatedAt => m.created_at_ms,
        SortKey::OccurredAt => m.occurred_at_ms,
    };
    all.sort_by(|a, b| {
        let o = key(a).cmp(&key(b)).then_with(|| a.id.cmp(&b.id));
        if q.descending { o.reverse() } else { o }
    });
    let total = all.len();
    let items: Vec<RawMemory> = all.into_iter().skip(offset).take(q.limit).collect();
    let next = offset + items.len();
    Ok(RawPage { items, cursor: if next < total { Some(format!("o:{next}")) } else { None } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{SearchMethod, SearchRequest};
    use serde_json::json;
    use std::sync::Arc;
    use xiar_store::keyring::{Keyring, MasterKey};
    use xiar_store::relational::RedbStore;
    use xiar_store::vector::EdgeVectorStore;

    fn mgr() -> (tempfile::TempDir, MemoryManager<RawMemory>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(EdgeVectorStore::open(dir.path(), "english").unwrap());
        let ring = Arc::new(Keyring::new(Arc::new(RedbStore::in_memory().unwrap()), MasterKey::generate()));
        (dir, MemoryManager::new(store, ring, &[]).unwrap())
    }

    fn raw(user: &str, ctx: &str, scope: &str, at: Ms) -> RawMemory {
        let mut r = RawMemory::new(user, ctx);
        r.filter_tags = json!({"scope": scope, "project": "alpha"}).as_object().cloned();
        r.created_at_ms = at;
        r.updated_at_ms = at;
        r.occurred_at_ms = at;
        r
    }

    #[test]
    fn append_replace_and_tag_modes_match_mirix() {
        let (_d, m) = mgr();
        let r = m.insert(raw("alice", "first note", "work", 1_000), BTreeMap::new()).unwrap();
        let u = update_raw(&m, "alice", &r.id, &RawUpdate { context: Some("second note".into()), context_mode: ContextMode::Append, ..Default::default() }, None).unwrap();
        assert_eq!(u.context, "first note\n\nsecond note");
        assert!(u.updated_at_ms > 1_000);
        assert_eq!(u.last_modify.operation, "updated");
        let u = update_raw(&m, "alice", &r.id, &RawUpdate { context: Some("only this".into()), ..Default::default() }, None).unwrap();
        assert_eq!(u.context, "only this");
        // merge keeps existing keys, replace drops them but preserves scope
        let u = update_raw(&m, "alice", &r.id, &RawUpdate { tags: json!({"priority": "high"}).as_object().cloned(), tags_mode: TagsMode::Merge, ..Default::default() }, None).unwrap();
        let t = u.filter_tags.unwrap();
        assert_eq!((t["scope"].as_str(), t["project"].as_str(), t["priority"].as_str()), (Some("work"), Some("alpha"), Some("high")));
        let u = update_raw(&m, "alice", &r.id, &RawUpdate { tags: json!({"only": "me"}).as_object().cloned(), tags_mode: TagsMode::Replace, ..Default::default() }, None).unwrap();
        let t = u.filter_tags.unwrap();
        assert_eq!((t.len(), t["scope"].as_str(), t["only"].as_str()), (2, Some("work"), Some("me")));
    }

    #[test]
    fn write_scope_is_enforced() {
        let (_d, m) = mgr();
        let r = m.insert(raw("alice", "note", "work", 1_000), BTreeMap::new()).unwrap();
        let wrong = RawUpdate { context: Some("x".into()), writer: Writer::Client { write_scope: Some("home".into()) }, ..Default::default() };
        let e = update_raw(&m, "alice", &r.id, &wrong, None).unwrap_err().to_string();
        assert!(e.contains("Access denied") && e.contains("'work'") && e.contains("'home'"), "{e}");
        let change = RawUpdate { tags: json!({"scope": "home"}).as_object().cloned(), tags_mode: TagsMode::Merge, writer: Writer::Client { write_scope: Some("work".into()) }, ..Default::default() };
        assert!(update_raw(&m, "alice", &r.id, &change, None).unwrap_err().to_string().contains("Cannot change memory scope"));
        let ok = RawUpdate { context: Some("fine".into()), writer: Writer::Client { write_scope: Some("work".into()) }, ..Default::default() };
        assert_eq!(update_raw(&m, "alice", &r.id, &ok, None).unwrap().context, "fine");
        assert!(matches!(update_raw(&m, "bob", &r.id, &ok, None), Err(MemError::NotFound(_))), "other users get not-found, not access-denied");
        assert_eq!(m.get("alice", &r.id, None).unwrap().unwrap().context, "fine");
    }

    #[test]
    fn a_client_without_a_write_scope_is_read_only() {
        let (_d, m) = mgr();
        let r = m.insert(raw("alice", "note", "work", 1_000), BTreeMap::new()).unwrap();
        let ro = RawUpdate { context: Some("x".into()), writer: Writer::Client { write_scope: None }, ..Default::default() };
        let e = update_raw(&m, "alice", &r.id, &ro, None).unwrap_err().to_string();
        assert!(e.contains("Access denied") && e.contains("has scope 'work', actor has write_scope 'None'"), "{e}");
        assert_eq!(m.get("alice", &r.id, None).unwrap().unwrap().context, "note");
        // the system itself is not a client and is not refused
        assert_eq!(update_raw(&m, "alice", &r.id, &RawUpdate { context: Some("by the system".into()), ..Default::default() }, None).unwrap().context, "by the system");
    }

    #[test]
    fn ttl_purge_is_strict_and_updates_extend_life() {
        let (_d, m) = mgr();
        let day = 24 * 3600 * 1000;
        let now = 100 * day;
        let old = m.insert(raw("alice", "old note", "work", now - 15 * day), BTreeMap::new()).unwrap();
        let edge = m.insert(raw("alice", "exactly fourteen days", "work", now - 14 * day), BTreeMap::new()).unwrap();
        let fresh = m.insert(raw("bob", "fresh note", "work", now - day), BTreeMap::new()).unwrap();
        assert_eq!(purge_expired(&m, now).unwrap(), 1, "only the 15-day-old item is strictly older than the cutoff");
        assert!(m.get("alice", &old.id, None).unwrap().is_none());
        assert!(m.get("alice", &edge.id, None).unwrap().is_some());
        assert!(m.get("bob", &fresh.id, None).unwrap().is_some());
        assert_eq!(purge_expired(&m, now + 2 * day).unwrap(), 1, "a day later the 14-day item expires too");
    }

    #[test]
    fn paged_listing_sort_cursor_and_filters() {
        let (_d, m) = mgr();
        for i in 0..7 {
            let mut r = raw("alice", &format!("note {i}"), if i % 2 == 0 { "work" } else { "home" }, 1_000 + i);
            r.updated_at_ms = 5_000 - i; // reverse of created order
            m.insert(r, BTreeMap::new()).unwrap();
        }
        let mut q = RawQuery { limit: 3, ..Default::default() };
        let p1 = list_raw(&m, "alice", &q).unwrap();
        assert_eq!(p1.items.iter().map(|r| r.context.as_str()).collect::<Vec<_>>(), vec!["note 0", "note 1", "note 2"], "default is -updated_at");
        q.cursor = p1.cursor;
        let p2 = list_raw(&m, "alice", &q).unwrap();
        q.cursor = p2.cursor;
        let p3 = list_raw(&m, "alice", &q).unwrap();
        assert_eq!((p2.items.len(), p3.items.len(), p3.cursor.is_none()), (3, 1, true));
        let (k, d) = parse_sort("created_at").unwrap();
        let asc = list_raw(&m, "alice", &RawQuery { sort: k, descending: d, limit: 2, ..Default::default() }).unwrap();
        assert_eq!(asc.items[0].context, "note 0");
        let (k, d) = parse_sort("-created_at").unwrap();
        assert_eq!(list_raw(&m, "alice", &RawQuery { sort: k, descending: d, limit: 1, ..Default::default() }).unwrap().items[0].context, "note 6");
        // scope + time range
        let home = list_raw(&m, "alice", &RawQuery { scopes: Some(vec!["home".into()]), limit: 100, ..Default::default() }).unwrap();
        assert_eq!(home.items.len(), 3);
        let tr = TimeRange { created_gte: Some(1_002), created_lte: Some(1_004), ..Default::default() };
        assert_eq!(list_raw(&m, "alice", &RawQuery { time_range: tr, limit: 100, ..Default::default() }).unwrap().items.len(), 3);
        assert!(list_raw(&m, "alice", &RawQuery { limit: 0, ..Default::default() }).is_err());
        assert!(list_raw(&m, "alice", &RawQuery { cursor: Some("garbage".into()), ..Default::default() }).is_err());
        assert!(parse_sort("name").is_err());
    }

    #[test]
    fn raw_search_by_text_still_works_after_updates() {
        let (_d, m) = mgr();
        let r = m.insert(raw("alice", "invoice totals for Q3", "work", 1_000), BTreeMap::new()).unwrap();
        update_raw(&m, "alice", &r.id, &RawUpdate { context: Some("and the budget review".into()), context_mode: ContextMode::Append, ..Default::default() }, None).unwrap();
        let mut q = SearchRequest::new("alice", "budget review");
        q.method = SearchMethod::Bm25;
        assert_eq!(m.search(&q).unwrap()[0].item.id, r.id);
    }
}
