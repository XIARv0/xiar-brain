// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
use crate::manager::*;
use crate::search::{Fusion, SearchMethod, SearchRequest};
use crate::types::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use xiar_store::keyring::{Keyring, MasterKey};
use xiar_store::relational::RedbStore;
use xiar_store::vector::{EdgeVectorStore, PayloadFilter, VectorStore};
use xiar_store::StoreError;

struct Env {
    _dir: tempfile::TempDir,
    store: Arc<EdgeVectorStore>,
    ring: Arc<Keyring>,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(EdgeVectorStore::open(dir.path(), "english").unwrap());
    let ring = Arc::new(Keyring::new(Arc::new(RedbStore::in_memory().unwrap()), MasterKey::generate()));
    Env { _dir: dir, store, ring }
}

fn episodic(e: &Env) -> MemoryManager<EpisodicEvent> {
    MemoryManager::new(e.store.clone(), e.ring.clone(), &[("summary", 4)]).unwrap()
}

fn emb(v: [f32; 4]) -> BTreeMap<String, Vec<f32>> {
    BTreeMap::from([("summary".to_string(), v.to_vec())])
}

fn ev(user: &str, summary: &str, details: &str, at: Ms, scope: &str) -> EpisodicEvent {
    let mut e = EpisodicEvent::new(user, "user_message", summary, details, "user", at);
    e.common.filter_tags = json!({"scope": scope, "tags": [scope, "all"]}).as_object().cloned();
    e
}

fn req(user: &str, q: &str, method: SearchMethod) -> SearchRequest {
    let mut r = SearchRequest::new(user, q);
    r.method = method;
    r
}

fn ids(r: &[Scored<EpisodicEvent>]) -> Vec<String> {
    r.iter().map(|s| s.item.summary.clone()).collect()
}

fn seeded(e: &Env) -> MemoryManager<EpisodicEvent> {
    let m = episodic(e);
    m.insert(ev("alice", "user prefers dark mode", "asked for a dark theme with green buttons", 1_000, "work"), emb([0.9, 0.1, 0.0, 0.0])).unwrap();
    m.insert(ev("alice", "quarterly invoice audit meeting", "discussed net amount column with Sarah", 2_000, "work"), emb([0.1, 0.9, 0.1, 0.0])).unwrap();
    m.insert(ev("alice", "dog named Max", "walks in the park every morning", 3_000, "home"), emb([0.0, 0.1, 0.1, 0.9])).unwrap();
    m.insert(ev("bob", "bob likes dark roast coffee", "ordered espresso", 4_000, "work"), emb([0.8, 0.0, 0.2, 0.1])).unwrap();
    m
}

#[test]
fn users_are_isolated_for_get_update_delete_and_search() {
    let e = env();
    let m = seeded(&e);
    let alice_item = m.list_recent("alice", 1, None, None).unwrap().remove(0);
    assert!(m.get("alice", &alice_item.common.id, None).unwrap().is_some());
    assert!(m.get("bob", &alice_item.common.id, None).unwrap().is_none(), "bob cannot read alice's item by id");
    assert!(m.update("bob", &alice_item.common.id, BTreeMap::new(), |x| EpisodicUpdate { summary: Some("hacked".into()), ..Default::default() }.apply(x)).unwrap().is_none());
    assert!(!m.delete("bob", &alice_item.common.id).unwrap());
    assert!(m.get("alice", &alice_item.common.id, None).unwrap().unwrap().summary != "hacked");
    let r = m.search(&req("bob", "dark", SearchMethod::Bm25)).unwrap();
    assert_eq!(ids(&r), vec!["bob likes dark roast coffee"], "bob only sees his own 'dark' item");
}

#[test]
fn update_keeps_existing_embeddings_and_reembeds_when_given() {
    let e = env();
    let m = episodic(&e);
    let item = m.insert(ev("alice", "original summary", "d", 1, "work"), emb([1.0, 0.0, 0.0, 0.0])).unwrap();
    // change only details: the summary vector must survive
    let (u, changed) = m.update("alice", &item.common.id, BTreeMap::new(), |x| EpisodicUpdate { details: Some("new details".into()), ..Default::default() }.apply(x)).unwrap().unwrap();
    assert_eq!(changed, vec!["details"]);
    assert_eq!(u.details, "new details");
    let mut r = req("alice", "", SearchMethod::Embedding);
    r.embedding = Some(vec![1.0, 0.0, 0.0, 0.0]);
    let hit = m.search(&r).unwrap();
    assert_eq!(hit.len(), 1);
    assert!(hit[0].score > 0.99, "vector was preserved: {}", hit[0].score);
    // change the summary and supply a new embedding: the old direction no longer matches
    m.update("alice", &item.common.id, emb([0.0, 1.0, 0.0, 0.0]), |x| EpisodicUpdate { summary: Some("changed".into()), ..Default::default() }.apply(x)).unwrap().unwrap();
    let hit = m.search(&r).unwrap();
    assert!(hit[0].score < 0.1, "re-embedded: {}", hit[0].score);
    // a no-op update does not rewrite anything
    let (_, none) = m.update("alice", &item.common.id, BTreeMap::new(), |_| vec![]).unwrap().unwrap();
    assert!(none.is_empty());
}

#[test]
fn all_search_methods() {
    let e = env();
    let m = seeded(&e);
    // bm25 across all fields, and restricted to one field
    assert_eq!(ids(&m.search(&req("alice", "invoice audit", SearchMethod::Bm25)).unwrap())[0], "quarterly invoice audit meeting");
    let mut r = req("alice", "Sarah", SearchMethod::Bm25);
    r.field = "details".into();
    assert_eq!(ids(&m.search(&r).unwrap())[0], "quarterly invoice audit meeting");
    // embedding
    let mut r = req("alice", "", SearchMethod::Embedding);
    r.embedding = Some(vec![0.85, 0.05, 0.05, 0.05]);
    assert_eq!(ids(&m.search(&r).unwrap())[0], "user prefers dark mode");
    r.embedding = None;
    assert!(m.search(&r).is_err(), "embedding search without an embedding is an error");
    // hybrid, and hybrid without embedding degrades to bm25
    let mut r = req("alice", "dark theme", SearchMethod::Hybrid);
    r.embedding = Some(vec![0.85, 0.05, 0.05, 0.05]);
    r.fusion = Fusion::Dbsf;
    assert_eq!(ids(&m.search(&r).unwrap())[0], "user prefers dark mode");
    r.embedding = None;
    assert_eq!(ids(&m.search(&r).unwrap())[0], "user prefers dark mode");
    // string_match: substring on cleaned text; fuzzy tolerates typos
    assert_eq!(ids(&m.search(&req("alice", "park every", SearchMethod::StringMatch)).unwrap()), vec!["dog named Max"]);
    assert!(m.search(&req("alice", "parkk evry", SearchMethod::StringMatch)).unwrap().is_empty());
    assert_eq!(ids(&m.search(&req("alice", "invoise audit", SearchMethod::FuzzyMatch)).unwrap()), vec!["quarterly invoice audit meeting"]);
    assert!(m.search(&req("alice", "zzzzzz qqqqq", SearchMethod::FuzzyMatch)).unwrap().is_empty());
    // empty query returns the most recent items first
    let recent = ids(&m.search(&req("alice", "", SearchMethod::Bm25)).unwrap());
    assert_eq!(recent, vec!["dog named Max", "quarterly invoice audit meeting", "user prefers dark mode"]);
    // limit and unknown field
    let mut r = req("alice", "", SearchMethod::Bm25);
    r.limit = 2;
    assert_eq!(m.search(&r).unwrap().len(), 2);
    r.field = "nonsense".into();
    assert!(m.search(&r).is_err());
}

#[test]
fn time_range_scopes_and_filter_tags_are_enforced() {
    let e = env();
    let m = seeded(&e);
    let mut r = req("alice", "", SearchMethod::Bm25);
    r.start_ms = Some(1_500);
    r.end_ms = Some(2_500);
    assert_eq!(ids(&m.search(&r).unwrap()), vec!["quarterly invoice audit meeting"], "time window on occurred_at");
    let mut r = req("alice", "dark invoice dog", SearchMethod::Bm25);
    r.start_ms = Some(500);
    r.end_ms = Some(1_500);
    assert_eq!(ids(&m.search(&r).unwrap()), vec!["user prefers dark mode"]);

    let mut r = req("alice", "dark invoice dog park", SearchMethod::Bm25);
    r.scopes = Some(vec!["home".into()]);
    assert_eq!(ids(&m.search(&r).unwrap()), vec!["dog named Max"]);
    r.scopes = Some(vec![]);
    assert!(m.search(&r).unwrap().is_empty(), "authorized for nothing sees nothing");
    r.scopes = None;
    assert_eq!(m.search(&r).unwrap().len(), 3);

    // $contains is evaluated after retrieval (post-filter) and still returns what matches
    let mut r = req("alice", "dark invoice dog park", SearchMethod::Bm25);
    r.filter_tags = json!({"tags": {"$contains": "home"}}).as_object().cloned();
    assert_eq!(ids(&m.search(&r).unwrap()), vec!["dog named Max"]);
    r.filter_tags = json!({"tags": {"$bogus": 1}}).as_object().cloned();
    assert!(matches!(m.search(&r), Err(MemError::Filter(_))));
    // string_match honours the same restrictions
    let mut r = req("alice", "meeting", SearchMethod::StringMatch);
    r.scopes = Some(vec!["home".into()]);
    assert!(m.search(&r).unwrap().is_empty());
    // get with scopes
    let item = m.list_recent("alice", 0, None, None).unwrap().into_iter().find(|x| x.summary.starts_with("dog")).unwrap();
    assert!(m.get("alice", &item.common.id, Some(&["work".to_string()])).unwrap().is_none());
    assert!(m.get("alice", &item.common.id, Some(&["home".to_string()])).unwrap().is_some());
}

#[test]
fn wide_post_filter_still_finds_rare_matches() {
    let e = env();
    let m = episodic(&e);
    for i in 0..60 {
        m.insert(ev("alice", &format!("common topic number {i}"), "filler", i, "work"), BTreeMap::new()).unwrap_or_else(|x| panic!("{x}"));
    }
    let mut rare = ev("alice", "common topic rare one", "filler", 100, "work");
    rare.common.filter_tags = json!({"scope": "work", "priority": "high"}).as_object().cloned();
    m.insert(rare, BTreeMap::new()).unwrap();
    let mut r = req("alice", "common topic", SearchMethod::Bm25);
    r.limit = 3;
    r.filter_tags = json!({"priority": "high"}).as_object().cloned();
    let got = m.search(&r).unwrap();
    assert_eq!(ids(&got), vec!["common topic rare one"], "over-fetch widens until the rare match is found");
}

#[test]
fn counts_and_erasure() {
    let e = env();
    let m = seeded(&e);
    assert_eq!(m.count("alice").unwrap(), 3);
    assert_eq!(m.count("bob").unwrap(), 1);
    assert_eq!(m.delete_by_user("alice").unwrap(), 3);
    assert_eq!(m.count("alice").unwrap(), 0);
    assert_eq!(m.count("bob").unwrap(), 1, "erasing alice leaves bob");
    let mut ev1 = ev("carol", "from the web client", "d", 1, "work");
    ev1.common.client_id = Some("client-web".into());
    m.insert(ev1, BTreeMap::new()).unwrap();
    assert_eq!(m.delete_by_client("client-web").unwrap(), 1);
    assert_eq!(m.count("carol").unwrap(), 0);
}

#[test]
fn rejects_bad_embeddings_and_invalid_items() {
    let e = env();
    let m = episodic(&e);
    assert!(matches!(m.insert(ev("a", "x", "y", 1, "w"), BTreeMap::from([("summary".to_string(), vec![1.0, 2.0])])), Err(MemError::Store(StoreError::DimensionMismatch { .. }))));
    assert!(m.insert(ev("a", "x", "y", 1, "w"), BTreeMap::from([("details".to_string(), vec![1.0; 4])])).is_err(), "details is not an embedded field here");
    assert!(matches!(m.insert(EpisodicEvent::new("a", "", "s", "d", "user", 1), BTreeMap::new()), Err(MemError::Validation(_))));
    let bad = MemoryManager::<EpisodicEvent>::new(e.store.clone(), e.ring.clone(), &[("nope", 4)]);
    assert!(matches!(bad, Err(MemError::Invalid(_))));
}

#[test]
fn knowledge_vault_secret_is_sealed_at_rest_and_unreadable_after_shredding() {
    let e = env();
    let m: MemoryManager<VaultItem> = MemoryManager::new(e.store.clone(), e.ring.clone(), &[("caption", 4)]).unwrap();
    let mut item = VaultItem::new("alice", "api_key", "user said so", "high", "sk-TOPSECRET-12345", "OpenAI API key for the finance bot");
    item.common.filter_tags = json!({"scope": "work"}).as_object().cloned();
    let id = item.common.id.clone();
    m.insert(item, BTreeMap::from([("caption".to_string(), vec![1.0, 0.0, 0.0, 0.0])])).unwrap();

    // what is actually on disk / in the index has no plaintext secret
    let raw = e.store.get("knowledge_vault", std::slice::from_ref(&id)).unwrap().remove(0).unwrap();
    let dump = serde_json::to_string(&raw).unwrap();
    assert!(!dump.contains("TOPSECRET") && !raw.contains_key("secret_value"), "plaintext must not be stored: {dump}");
    assert!(raw.contains_key("secret_sealed"));

    // owner reads it back in full
    assert_eq!(m.get("alice", &id, None).unwrap().unwrap().secret_value, "sk-TOPSECRET-12345");
    // the secret is not searchable by text
    assert!(m.search(&req("alice", "TOPSECRET", SearchMethod::Bm25)).unwrap().is_empty());
    assert_eq!(m.search(&req("alice", "finance OpenAI key", SearchMethod::Bm25)).unwrap().len(), 1, "but the caption is");

    // erasure: crypto-shred the user and the secret is gone for good
    e.ring.shred("alice").unwrap();
    let err = m.get("alice", &id, None).unwrap_err();
    assert!(matches!(err, MemError::Store(StoreError::KeyErased(_))), "{err}");
    // the raw vector-store row was NOT the only copy of the secret: nothing else can recover it
    assert!(!serde_json::to_string(&e.store.get("knowledge_vault", &[id]).unwrap()).unwrap().contains("TOPSECRET"));
    // untouched control: another user's vault is unaffected
    let mut other = VaultItem::new("bob", "bookmark", "s", "low", "https://example.com", "a bookmark");
    other.common.filter_tags = None;
    let oid = other.common.id.clone();
    m.insert(other, BTreeMap::new()).unwrap();
    assert_eq!(m.get("bob", &oid, None).unwrap().unwrap().secret_value, "https://example.com");
    assert!(e.store.count("knowledge_vault", &PayloadFilter::eq("user_id", "bob")).unwrap() == 1);
}

#[test]
fn raw_and_other_layers_work_through_the_same_manager() {
    let e = env();
    let raw: MemoryManager<RawMemory> = MemoryManager::new(e.store.clone(), e.ring.clone(), &[]).unwrap();
    let r = raw.insert(RawMemory::new("alice", "user asked about invoice totals for Q3"), BTreeMap::new()).unwrap();
    assert_eq!(raw.search(&req("alice", "invoice totals", SearchMethod::Bm25)).unwrap()[0].item.id, r.id);
    let sem: MemoryManager<SemanticItem> = MemoryManager::new(e.store.clone(), e.ring.clone(), &[("name", 4), ("summary", 4), ("details", 4)]).unwrap();
    sem.insert(SemanticItem::new("alice", "Sarah", "colleague in finance", "works on audits", "conversation"), BTreeMap::new()).unwrap();
    assert_eq!(sem.search(&req("alice", "audits finance", SearchMethod::Bm25)).unwrap()[0].item.name, "Sarah");
    let proc_: MemoryManager<ProceduralItem> = MemoryManager::new(e.store.clone(), e.ring.clone(), &[]).unwrap();
    let mut p = ProceduralItem::new("alice", "invoice-audit", "workflow", "check invoices", "1. open csv 2. compare net amount");
    p.triggers = vec!["invoice audit".into()];
    proc_.insert(p, BTreeMap::new()).unwrap();
    assert_eq!(proc_.search(&req("alice", "compare net amount", SearchMethod::Bm25)).unwrap()[0].item.name, "invoice-audit");
    let res: MemoryManager<ResourceItem> = MemoryManager::new(e.store.clone(), e.ring.clone(), &[]).unwrap();
    res.insert(ResourceItem::new("alice", "Q3 report", "quarterly numbers", "markdown", "# Q3\nrevenue grew"), BTreeMap::new()).unwrap();
    assert_eq!(res.search(&req("alice", "revenue grew", SearchMethod::Bm25)).unwrap()[0].item.title, "Q3 report");
}

#[test]
fn list_with_any_finds_items_by_a_listed_payload_value_for_one_user_in_scope() {
    let e = env();
    let m: MemoryManager<SemanticItem> = MemoryManager::new(e.store.clone(), e.ring.clone(), &[]).unwrap();
    let mk = |user: &str, name: &str, types: &[&str], scope: &str, at: Ms| {
        let mut s = SemanticItem::new(user, name, "summary", "details", "chat");
        s.entity_types = types.iter().map(|t| t.to_string()).collect();
        s.common.created_at_ms = at;
        s.common.filter_tags = json!({"scope": scope, "tags": [scope]}).as_object().cloned();
        m.insert(s, BTreeMap::new()).unwrap();
    };
    mk("alice", "speedster", &["SportsCar"], "home", 1_000);
    mk("alice", "family car", &["Car", "Insured"], "home", 2_000);
    mk("alice", "sofa", &["Furniture"], "home", 3_000);
    mk("alice", "work van", &["Van"], "work", 4_000);
    mk("bob", "bob's car", &["Car"], "home", 5_000);
    let any = |user: &str, vals: &[&str], limit: usize, scopes: Option<&[String]>| -> Vec<String> {
        let v: Vec<String> = vals.iter().map(|s| s.to_string()).collect();
        m.list_with_any(user, "entity_types", &v, limit, scopes, None).unwrap().into_iter().map(|i| i.name).collect()
    };
    assert_eq!(any("alice", &["Car", "SportsCar"], 0, None), ["family car", "speedster"], "a value inside a list matches, newest first");
    assert_eq!(any("alice", &["Insured"], 0, None), ["family car"], "any element of the item's list counts");
    assert_eq!(any("alice", &["Car"], 0, None), ["family car"], "another user's items never appear");
    assert_eq!(any("bob", &["Car"], 0, None), ["bob's car"]);
    assert_eq!(any("alice", &["Car", "SportsCar", "Van"], 2, None), ["work van", "family car"], "the limit keeps the newest");
    assert_eq!(any("alice", &["Car", "Van"], 0, Some(&["home".to_string()])), ["family car"], "scopes apply");
    assert!(any("alice", &["Car"], 0, Some(&[])).is_empty(), "an empty scope list matches nothing");
    assert!(any("alice", &[], 0, None).is_empty(), "no values, no items");
    assert!(any("alice", &["Boat"], 0, None).is_empty());
}

/// Timing, not a rule: `cargo test -p xiar-memory --release type_lookup_time -- --ignored --nocapture`
#[test]
#[ignore]
fn type_lookup_time_over_twenty_thousand_items() {
    let e = env();
    let m: MemoryManager<SemanticItem> = MemoryManager::new(e.store.clone(), e.ring.clone(), &[]).unwrap();
    let t = std::time::Instant::now();
    for i in 0..20_000 {
        let mut s = SemanticItem::new("alice", &format!("thing {i}"), "summary", "details", "chat");
        s.entity_types = vec![format!("Type{}", i % 500)];
        m.insert(s, BTreeMap::new()).unwrap();
    }
    println!("inserted 20000 in {:?}", t.elapsed());
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let got = m.list_with_any("alice", "entity_types", &["Type7".to_string()], 0, None, None).unwrap();
        println!("lookup of 40 matches among 20000: {:?} ({} found)", t.elapsed(), got.len());
    }
}


#[test]
fn entity_types_are_bm25_searchable_for_class_queries() {
    let e = env();
    let m: MemoryManager<SemanticItem> = MemoryManager::new(e.store.clone(), e.ring.clone(), &[]).unwrap();
    let mut s = SemanticItem::new("alice", "Sensor Supply Co", "supplies sensors", "vendor partner", "org_chart");
    s.entity_types = vec!["Vendor".into()];
    m.insert(s, BTreeMap::new()).unwrap();
    let mut s = SemanticItem::new("alice", "Daniel Reyes", "Director of Operations", "leads ops", "org_chart");
    s.entity_types = vec!["Director".into()];
    m.insert(s, BTreeMap::new()).unwrap();
    let r = m.search(&req("alice", "Vendor", SearchMethod::Bm25)).unwrap();
    assert_eq!(r[0].item.name, "Sensor Supply Co", "class token Vendor must hit typed memory");
}

