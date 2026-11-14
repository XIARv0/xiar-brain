// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
use crate::hub::MemoryHub;
use crate::manager::MemError;
use crate::search::{SearchMethod, SearchRequest};
use crate::types::*;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use xiar_store::keyring::{Keyring, MasterKey};
use xiar_store::relational::RedbStore;
use xiar_store::vector::EdgeVectorStore;
use xiar_store::StoreError;

struct Env {
    dir: tempfile::TempDir,
    hub: MemoryHub,
    ring: Arc<Keyring>,
}

fn env(dim: Option<u32>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let vectors = Arc::new(EdgeVectorStore::open(&dir.path().join("vectors"), "english").unwrap());
    let rel = Arc::new(RedbStore::in_memory().unwrap());
    let ring = Arc::new(Keyring::new(rel.clone(), MasterKey::generate()));
    let hub = MemoryHub::new(vectors, rel, ring.clone(), dim).unwrap();
    Env { dir, hub, ring }
}

fn emb(fields: &[&str], dim: usize, hot: usize) -> BTreeMap<String, Vec<f32>> {
    fields.iter().map(|f| (f.to_string(), (0..dim).map(|i| if i == hot { 1.0 } else { 0.0 }).collect())).collect()
}

fn populate(h: &MemoryHub, user: &str, marker: &str) {
    h.core.ensure_defaults(user, "", &[("human", &format!("{user} likes {marker}")), ("persona", "helpful")]).unwrap();
    h.episodic.insert(EpisodicEvent::new(user, "user_message", &format!("{marker} event"), "details", "user", 1_000), emb(&["summary", "details"], 4, 0)).unwrap();
    h.semantic.insert(SemanticItem::new(user, &format!("{marker} concept"), "summary", "details", "chat"), emb(&["name", "summary", "details"], 4, 1)).unwrap();
    h.procedural.insert(ProceduralItem::new(user, &format!("{marker}-skill"), "workflow", "desc", "do things"), emb(&["description", "instructions"], 4, 2)).unwrap();
    h.resource.insert(ResourceItem::new(user, &format!("{marker} doc"), "sum", "markdown", "content body"), emb(&["summary"], 4, 3)).unwrap();
    h.vault.insert(VaultItem::new(user, "api_key", "user", "high", &format!("sk-{marker}-SECRET"), &format!("{marker} key")), emb(&["caption"], 4, 0)).unwrap();
    h.raw.insert(RawMemory::new(user, &format!("raw {marker} context")), emb(&["context"], 4, 1)).unwrap();
    let mut x = SkillExperience::new(user, "s1", ExperienceType::WorthLearning, &format!("{marker} lesson"), "c", 0.5, 0.5, "{}");
    x.agent_id = Some("agent-1".into());
    h.experience.create(x).unwrap();
}

#[test]
fn export_covers_every_layer_and_redacts_secrets_by_default() {
    let e = env(Some(4));
    populate(&e.hub, "alice", "alpha");
    populate(&e.hub, "bob", "beta");
    let x = e.hub.export_user("alice", false).unwrap();
    assert_eq!((x.core_blocks.len(), x.episodic.len(), x.semantic.len(), x.procedural.len(), x.resource.len(), x.knowledge_vault.len(), x.raw.len(), x.skill_experiences.len()), (2, 1, 1, 1, 1, 1, 1, 1));
    let json = x.to_json();
    assert!(json.contains("alpha") && !json.contains("beta"), "only alice's data");
    assert!(!json.contains("SECRET") && json.contains("<redacted>"), "secrets redacted by default");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["format"], "xiar-user-export-v1");
    let with = e.hub.export_user("alice", true).unwrap().to_json();
    assert!(with.contains("sk-alpha-SECRET"), "explicit include_secrets returns them");
    let md = x.to_markdown();
    assert!(md.contains("# Data export for alice") && md.contains("## Episodic memory (1)") && md.contains("alpha event"));
    assert!(!md.contains("SECRET"));
}

#[test]
fn erase_removes_everything_for_one_user_and_destroys_the_key() {
    let e = env(Some(4));
    populate(&e.hub, "alice", "alpha");
    populate(&e.hub, "bob", "beta");
    let report = e.hub.erase_user("alice").unwrap();
    assert_eq!((report.episodic, report.semantic, report.procedural, report.resource, report.knowledge_vault, report.raw, report.core_blocks, report.skill_experiences), (1, 1, 1, 1, 1, 1, 2, 1));
    assert!(report.key_destroyed);
    assert_eq!(report.total_items(), 9);
    // alice is gone everywhere
    let after = e.hub.export_user("alice", true);
    assert!(after.is_err() || after.as_ref().unwrap().episodic.is_empty(), "nothing left to export");
    assert_eq!(e.hub.episodic.count("alice").unwrap(), 0);
    assert!(!e.ring.has_key("alice").unwrap());
    // bob is untouched, including his secret
    let bob = e.hub.export_user("bob", true).unwrap();
    assert_eq!((bob.episodic.len(), bob.core_blocks.len(), bob.skill_experiences.len()), (1, 2, 1));
    assert!(bob.to_json().contains("sk-beta-SECRET"));
    // erasing again is harmless
    let again = e.hub.erase_user("alice").unwrap();
    assert_eq!(again.total_items(), 0);
}

#[test]
fn erased_content_is_not_left_in_the_vector_store_files() {
    let e = env(Some(4));
    populate(&e.hub, "alice", "ZEBRA9137MARKER");
    populate(&e.hub, "bob", "KEEPME4242MARKER");
    // flush so the data is definitely on disk, and prove the marker IS findable before erasure
    for k in MemoryKind::ALL {
        e.hub.episodic.count("alice").unwrap();
        let _ = k;
    }
    let before = find_marker(&e.dir.path().join("vectors"), b"ZEBRA9137MARKER");
    assert!(before > 0, "sanity: the scan must be able to see plaintext payloads before erasure");
    e.hub.erase_user("alice").unwrap();
    let after = find_marker(&e.dir.path().join("vectors"), b"ZEBRA9137MARKER");
    let kept = find_marker(&e.dir.path().join("vectors"), b"KEEPME4242MARKER");
    assert!(kept > 0, "bob's data must still be there");
    assert_eq!(after, 0, "alice's plaintext must be physically gone from the vector store files after erase + compaction");
}

/// number of files under `dir` that contain `needle`
fn find_marker(dir: &Path, needle: &[u8]) -> usize {
    let mut n = 0;
    for entry in walk(dir) {
        if let Ok(bytes) = std::fs::read(&entry) {
            if bytes.windows(needle.len()).any(|w| w == needle) {
                n += 1;
            }
        }
    }
    n
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { out.extend(walk(&p)); } else { out.push(p); }
        }
    }
    out
}

#[test]
fn text_only_mode_works_without_an_embedding_model() {
    let e = env(None);
    e.hub.episodic.insert(EpisodicEvent::new("alice", "user_message", "booked flight to Lisbon", "d", "user", 1), BTreeMap::new()).unwrap();
    let mut q = SearchRequest::new("alice", "Lisbon flight");
    q.method = SearchMethod::Bm25;
    assert_eq!(e.hub.episodic.search(&q).unwrap().len(), 1);
    q.method = SearchMethod::Embedding;
    q.embedding = Some(vec![1.0; 4]);
    assert!(matches!(e.hub.episodic.search(&q), Err(MemError::Invalid(_))), "no embeddings configured");
    assert!(matches!(
        e.hub.episodic.insert(EpisodicEvent::new("alice", "t", "s", "d", "user", 1), emb(&["summary"], 4, 0)),
        Err(MemError::Invalid(_))
    ));
    let _ = StoreError::Decrypt;
}

#[test]
fn an_empty_memory_system_stays_small_on_disk() {
    // Upstream Edge preallocates 32 MiB per store/WAL (non-sparse on NTFS): an empty hub was ~1.2 GB.
    let e = env(Some(768));
    populate_text_only(&e.hub);
    let total: u64 = walk(&e.dir.path().join("vectors")).iter().map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
    println!("empty-ish hub: {} MB on disk", total / 1_000_000);
    assert!(total < 80_000_000, "vector store footprint regressed: {} MB", total / 1_000_000);
}

fn populate_text_only(h: &MemoryHub) {
    h.episodic.insert(EpisodicEvent::new("alice", "user_message", "hello", "world", "user", 1), BTreeMap::new()).unwrap();
}

// ------------------------------------------------------------------ import (restore from a backup)
fn fake_embedder(texts: &[String]) -> Option<Vec<Vec<f32>>> {
    Some(texts.iter().map(|t| (0..4).map(|i| if i == t.len() % 4 { 1.0 } else { 0.0 }).collect()).collect())
}

fn meta() -> EmbeddingMeta {
    EmbeddingMeta { plugin: "test".into(), model: "fake".into(), dim: 4 }
}

#[test]
fn a_backup_restores_into_a_fresh_system_and_is_searchable() {
    let src = env(Some(4));
    populate(&src.hub, "alice", "alpha");
    let exported = src.hub.export_user("alice", true).unwrap();
    let json = exported.to_json();
    let parsed: crate::hub::UserExport = serde_json::from_str(&json).unwrap();

    let dst = env(Some(4));
    let re = crate::hub::ReEmbedder { embed: &fake_embedder, meta: meta() };
    let rep = dst.hub.import_user(&parsed, Some(&re)).unwrap();
    assert!(rep.failed.is_empty(), "{:?}", rep.failed);
    assert_eq!((rep.imported, rep.skipped_redacted), (9, 0));
    assert!(rep.embedded >= 5);
    let back = dst.hub.export_user("alice", true).unwrap();
    assert_eq!(back.episodic.iter().map(|e| (&e.common.id, &e.summary)).collect::<Vec<_>>(), exported.episodic.iter().map(|e| (&e.common.id, &e.summary)).collect::<Vec<_>>(), "ids and text survive");
    assert_eq!(back.core_blocks.len(), 2);
    assert_eq!(back.knowledge_vault[0]["secret_value"], "sk-alpha-SECRET");
    assert_eq!(back.skill_experiences.len(), 1);
    assert!(back.episodic[0].embedding.is_some(), "re-embedded items record the model");
    // importing twice replaces, never duplicates
    dst.hub.import_user(&parsed, Some(&re)).unwrap();
    assert_eq!(dst.hub.export_user("alice", true).unwrap().episodic.len(), 1);
}

#[test]
fn a_redacted_export_skips_secrets_and_text_only_import_still_works() {
    let src = env(Some(4));
    populate(&src.hub, "alice", "alpha");
    let parsed: crate::hub::UserExport = serde_json::from_str(&src.hub.export_user("alice", false).unwrap().to_json()).unwrap();
    let dst = env(None);
    let rep = dst.hub.import_user(&parsed, None).unwrap();
    assert_eq!((rep.imported, rep.skipped_redacted, rep.embedded), (8, 1, 0));
    assert!(rep.failed.is_empty(), "{:?}", rep.failed);
    assert!(dst.hub.export_user("alice", true).unwrap().knowledge_vault.is_empty());
}

#[test]
fn an_import_refuses_other_users_items_and_unknown_formats() {
    let src = env(Some(4));
    populate(&src.hub, "alice", "alpha");
    let mut x: crate::hub::UserExport = serde_json::from_str(&src.hub.export_user("alice", true).unwrap().to_json()).unwrap();
    let dst = env(Some(4));
    let mut tampered = x.clone();
    tampered.user = "mallory".into();
    let rep = dst.hub.import_user(&tampered, None).unwrap();
    assert_eq!(rep.imported, 0, "items belonging to alice are not planted into another account");
    assert_eq!(rep.failed.len(), 9);
    x.format = "something-else".into();
    assert!(dst.hub.import_user(&x, None).is_err());
}

#[test]
fn sources_and_the_references_to_them_are_exported_restored_and_erased_with_the_user() {
    use crate::sources::{SourceKind, SourceRecord};
    let e = env(None);
    let src = e.hub.sources.create(SourceRecord::new("alice", SourceKind::Screenshot, "Chrome").with_url("https://docs.rs/x?key=secret")).unwrap();
    let other = e.hub.sources.create(SourceRecord::new("bob", SourceKind::Document, "Files").with_label("b.pdf")).unwrap();
    let mut item = SemanticItem::new("alice", "docs", "a page", "details", "screen");
    item.common.source_refs = vec![src.id.clone()];
    e.hub.semantic.insert(item, BTreeMap::new()).unwrap();

    let x = e.hub.export_user("alice", false).unwrap();
    assert_eq!(x.sources.len(), 1, "only the user's own sources are exported");
    assert_eq!(x.sources[0].url.as_deref(), Some("https://docs.rs/x"), "the stored address has no query string");
    assert_eq!(x.semantic[0].common.source_refs, [src.id.clone()]);
    assert!(x.to_markdown().contains("Sources (1)") && x.to_markdown().contains("Chrome: docs.rs"));
    assert!(!x.to_json().contains("key=secret"));

    // a fresh system gets both back, and importing twice does not duplicate
    let fresh = env(None);
    let r = fresh.hub.import_user(&x, None).unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    fresh.hub.import_user(&x, None).unwrap();
    assert_eq!(fresh.hub.sources.count("alice").unwrap(), 1);
    assert_eq!(fresh.hub.sources.describe("alice", std::slice::from_ref(&src.id)), " [Sources: Chrome: docs.rs]");

    // an export that names another user's source is refused for that record
    let mut bad = x.clone();
    bad.sources.push(other.clone());
    let r = fresh.hub.import_user(&bad, None).unwrap();
    assert!(r.failed.iter().any(|f| f.contains("belongs to another user")), "{:?}", r.failed);
    assert_eq!(fresh.hub.sources.count("bob").unwrap(), 0);

    let report = e.hub.erase_user("alice").unwrap();
    assert_eq!(report.sources, 1);
    assert_eq!(e.hub.sources.count("alice").unwrap(), 0);
    assert_eq!(e.hub.sources.count("bob").unwrap(), 1, "another user's sources are untouched");
}

