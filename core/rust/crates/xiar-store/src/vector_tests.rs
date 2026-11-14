// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
use crate::vector::*;
use crate::StoreError;
use serde_json::json;

fn spec() -> CollectionSpec {
    CollectionSpec { name: "episodic".into(), dense: vec![("summary".into(), 4)], text_fields: vec!["summary".into(), "details".into()] }
}

fn pt(id: &str, user: &str, summary: &str, details: &str, v: Option<[f32; 4]>, ts: i64, scope: &str) -> VecPoint {
    let mut p = VecPoint { id: id.into(), ..Default::default() };
    p.payload = json!({"user_id": user, "summary": summary, "details": details, "occurred_at_ms": ts, "filter_tags": {"scope": scope}}).as_object().unwrap().clone();
    p.text.insert("summary".into(), summary.into());
    p.text.insert("details".into(), details.into());
    if let Some(v) = v {
        p.dense.insert("summary".into(), v.to_vec());
    }
    p
}

fn seeded() -> (tempfile::TempDir, EdgeVectorStore) {
    let dir = tempfile::tempdir().unwrap();
    let s = EdgeVectorStore::open(dir.path(), "english").unwrap();
    s.ensure(&spec()).unwrap();
    s.upsert(
        "episodic",
        vec![
            pt("ep_mem-1", "alice", "user prefers dark mode", "asked for a dark theme with green buttons", Some([0.9, 0.1, 0.0, 0.0]), 1_000, "work"),
            pt("ep_mem-2", "alice", "quarterly invoice audit meeting", "discussed net amount column with Sarah", Some([0.1, 0.9, 0.1, 0.0]), 2_000, "work"),
            pt("ep_mem-3", "alice", "dog named Max", "walks in the park every morning", Some([0.0, 0.1, 0.1, 0.9]), 3_000, "home"),
            pt("ep_mem-4", "bob", "bob likes dark roast coffee", "ordered espresso", Some([0.8, 0.0, 0.2, 0.1]), 4_000, "work"),
            pt("ep_mem-5", "alice", "no embedding available for this one", "stored while the embedder was offline", None, 5_000, "work"),
        ],
    )
    .unwrap();
    (dir, s)
}

fn ids(h: &[Hit]) -> Vec<String> {
    h.iter().map(|x| x.id.clone()).collect()
}

#[test]
fn bm25_dense_and_hybrid_with_user_isolation() {
    let (_d, s) = seeded();
    let alice = PayloadFilter::eq("user_id", "alice");
    let bm = s.query("episodic", &Query::Bm25 { field: None, text: "invoice audit".into() }, &alice, 3, None).unwrap();
    assert_eq!(bm[0].id, "ep_mem-2");
    let dn = s.query("episodic", &Query::Dense { vector_name: "summary".into(), vector: vec![0.85, 0.05, 0.05, 0.05] }, &alice, 3, None).unwrap();
    assert_eq!(dn[0].id, "ep_mem-1");
    assert!(!ids(&dn).contains(&"ep_mem-4".to_string()), "bob's memory must never appear for alice");
    for fusion in [FusionKind::Rrf, FusionKind::Dbsf, FusionKind::Weighted(2.0, 1.0)] {
        let h = s
            .query(
                "episodic",
                &Query::Hybrid { vector_name: "summary".into(), vector: vec![0.85, 0.05, 0.05, 0.05], field: None, text: "dark theme".into(), fusion },
                &alice,
                3,
                None,
            )
            .unwrap();
        assert_eq!(h[0].id, "ep_mem-1", "{fusion:?}");
        assert!(h.iter().all(|x| x.payload["user_id"] == "alice"));
    }
}

#[test]
fn field_restricted_bm25_only_matches_that_field() {
    let (_d, s) = seeded();
    let alice = PayloadFilter::eq("user_id", "alice");
    // "Sarah" appears only in details
    let in_details = s.query("episodic", &Query::Bm25 { field: Some("details".into()), text: "Sarah".into() }, &alice, 5, None).unwrap();
    assert_eq!(in_details.first().map(|h| h.id.as_str()), Some("ep_mem-2"));
    let in_summary = s.query("episodic", &Query::Bm25 { field: Some("summary".into()), text: "Sarah".into() }, &alice, 5, None).unwrap();
    assert!(in_summary.iter().all(|h| h.score <= 0.0), "Sarah is not in any summary: {in_summary:?}");
    assert!(s.query("episodic", &Query::Bm25 { field: Some("nope".into()), text: "x".into() }, &alice, 5, None).is_err());
}

#[test]
fn filters_eq_any_range_and_must_not() {
    let (_d, s) = seeded();
    let q = Query::Bm25 { field: None, text: "dark theme invoice dog embedding".into() };
    let work = PayloadFilter::eq("user_id", "alice").and(Cond::Eq("filter_tags.scope".into(), json!("work")));
    let h = s.query("episodic", &q, &work, 10, None).unwrap();
    assert!(h.iter().all(|x| x.payload["filter_tags"]["scope"] == "work"));
    assert!(!ids(&h).contains(&"ep_mem-3".to_string()));
    let any = PayloadFilter::eq("user_id", "alice").and(Cond::Any("filter_tags.scope".into(), vec![json!("home"), json!("nowhere")]));
    assert_eq!(ids(&s.query("episodic", &q, &any, 10, None).unwrap()), vec!["ep_mem-3"]);
    let range = PayloadFilter::eq("user_id", "alice").and(Cond::Range { key: "occurred_at_ms".into(), gte: Some(1_500.0), lte: Some(3_000.0) });
    let mut r = ids(&s.query("episodic", &q, &range, 10, None).unwrap());
    r.sort();
    assert_eq!(r, vec!["ep_mem-2", "ep_mem-3"]);
    let not = PayloadFilter { must: vec![Cond::Eq("user_id".into(), json!("alice"))], must_not: vec![Cond::Eq("filter_tags.scope".into(), json!("work"))] };
    assert_eq!(ids(&s.query("episodic", &q, &not, 10, None).unwrap()), vec!["ep_mem-3"]);
}

#[test]
fn points_without_embeddings_are_still_found_by_text() {
    let (_d, s) = seeded();
    let h = s.query("episodic", &Query::Bm25 { field: None, text: "embedder offline".into() }, &PayloadFilter::eq("user_id", "alice"), 3, None).unwrap();
    assert_eq!(h[0].id, "ep_mem-5");
}

#[test]
fn get_scroll_count_delete() {
    let (_d, s) = seeded();
    let got = s.get("episodic", &["ep_mem-2".into(), "ep_mem-nope".into()]).unwrap();
    assert_eq!(got[0].as_ref().unwrap()["summary"], "quarterly invoice audit meeting");
    assert!(got[1].is_none());
    let alice = PayloadFilter::eq("user_id", "alice");
    assert_eq!(s.count("episodic", &alice).unwrap(), 4);
    let mut seen = vec![];
    let mut cursor: Option<String> = None;
    loop {
        let (page, next) = s.scroll("episodic", &alice, 3, cursor.as_deref()).unwrap();
        seen.extend(page.iter().map(|p| p["_id"].as_str().unwrap().to_string()));
        match next {
            Some(n) => cursor = Some(n),
            None => break,
        }
    }
    seen.sort();
    assert_eq!(seen, vec!["ep_mem-1", "ep_mem-2", "ep_mem-3", "ep_mem-5"], "paging returns every alice point exactly once");
    s.delete("episodic", &["ep_mem-1".into()]).unwrap();
    assert!(s.get("episodic", &["ep_mem-1".into()]).unwrap()[0].is_none());
    assert_eq!(s.delete_where("episodic", &alice).unwrap(), 3);
    assert_eq!(s.count("episodic", &alice).unwrap(), 0);
    assert_eq!(s.count("episodic", &PayloadFilter::eq("user_id", "bob")).unwrap(), 1, "other users untouched");
    assert!(s.delete_where("episodic", &PayloadFilter::default()).is_err(), "empty filter must be refused");
}

#[test]
fn upsert_replaces_and_data_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let s = EdgeVectorStore::open(dir.path(), "english").unwrap();
        s.ensure(&spec()).unwrap();
        s.upsert("episodic", vec![pt("ep_mem-1", "alice", "first version", "d", Some([1.0, 0.0, 0.0, 0.0]), 1, "work")]).unwrap();
        s.upsert("episodic", vec![pt("ep_mem-1", "alice", "second version", "d", Some([1.0, 0.0, 0.0, 0.0]), 1, "work")]).unwrap();
        s.flush("episodic").unwrap();
    }
    let s = EdgeVectorStore::open(dir.path(), "english").unwrap();
    s.ensure(&spec()).unwrap();
    assert_eq!(s.count("episodic", &PayloadFilter::eq("user_id", "alice")).unwrap(), 1, "same id = same point");
    assert_eq!(s.get("episodic", &["ep_mem-1".into()]).unwrap()[0].as_ref().unwrap()["summary"], "second version");
    let h = s.query("episodic", &Query::Bm25 { field: None, text: "second".into() }, &PayloadFilter::eq("user_id", "alice"), 3, None).unwrap();
    assert_eq!(h[0].id, "ep_mem-1");
}

#[test]
fn dimension_and_schema_errors_are_clear() {
    let (_d, s) = seeded();
    let mut bad = pt("ep_mem-9", "alice", "x", "y", None, 1, "work");
    bad.dense.insert("summary".into(), vec![1.0, 2.0]);
    assert!(matches!(s.upsert("episodic", vec![bad]), Err(StoreError::DimensionMismatch { expected: 4, got: 2, .. })));
    let mut unknown = pt("ep_mem-9", "alice", "x", "y", None, 1, "work");
    unknown.dense.insert("nope".into(), vec![1.0; 4]);
    assert!(s.upsert("episodic", vec![unknown]).is_err());
    assert!(matches!(
        s.query("episodic", &Query::Dense { vector_name: "summary".into(), vector: vec![1.0] }, &PayloadFilter::default(), 1, None),
        Err(StoreError::DimensionMismatch { .. })
    ));
    assert!(s.upsert("missing", vec![]).is_err(), "must call ensure first");
    let other = CollectionSpec { dense: vec![("summary".into(), 8)], ..spec() };
    assert!(s.ensure(&other).is_err(), "same collection, different schema while open");
}

#[test]
fn reopening_with_a_different_dimension_is_refused_not_corrupted() {
    let dir = tempfile::tempdir().unwrap();
    {
        let s = EdgeVectorStore::open(dir.path(), "english").unwrap();
        s.ensure(&spec()).unwrap();
        s.upsert("episodic", vec![pt("ep_mem-1", "alice", "a", "b", Some([1.0, 0.0, 0.0, 0.0]), 1, "work")]).unwrap();
        s.flush("episodic").unwrap();
    }
    let s = EdgeVectorStore::open(dir.path(), "english").unwrap();
    let e = s.ensure(&CollectionSpec { dense: vec![("summary".into(), 768)], ..spec() }).unwrap_err();
    assert!(e.to_string().contains("re-index"), "{e}");
}

#[test]
fn get_dense_returns_vectors_and_omits_missing_ones() {
    let (_d, s) = seeded();
    let got = s.get_dense("episodic", &["ep_mem-1".into(), "ep_mem-5".into(), "ep_mem-nope".into()]).unwrap();
    let (p, v) = got[0].as_ref().unwrap();
    assert_eq!(p["_id"], "ep_mem-1");
    // cosine collections store unit-length vectors; re-upserting them is score-equivalent
    let n = (0.9f32 * 0.9 + 0.1 * 0.1).sqrt();
    let want = [0.9 / n, 0.1 / n, 0.0, 0.0];
    assert!(v["summary"].iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5), "{:?}", v["summary"]);
    assert!(got[1].as_ref().unwrap().1.is_empty(), "point stored without an embedding has no dense vectors");
    assert!(got[2].is_none());
}

#[test]
fn compact_rebuilds_and_keeps_live_data_searchable() {
    let (d, s) = seeded();
    s.ensure_keyword_index("episodic", "filter_tags.scope").unwrap();
    s.ensure_keyword_index("episodic", "filter_tags.scope").unwrap(); // a second call is harmless
    let work_before = s.count("episodic", &PayloadFilter::eq("user_id", "alice").and(Cond::Eq("filter_tags.scope".into(), "work".into()))).unwrap();
    assert!(work_before > 0, "the seeded data has alice items in scope work");
    s.delete_where("episodic", &PayloadFilter::eq("user_id", "bob")).unwrap();
    s.compact("episodic").unwrap();
    // data of the remaining user is intact and fully searchable, vectors included
    let alice = PayloadFilter::eq("user_id", "alice");
    assert_eq!(s.count("episodic", &alice).unwrap(), 4);
    s.ensure_keyword_index("episodic", "filter_tags.scope").unwrap(); // the rebuilt shard has its index again, and still answers
    assert_eq!(s.count("episodic", &alice.clone().and(Cond::Eq("filter_tags.scope".into(), "work".into()))).unwrap(), work_before, "the same matches through the index after the rebuild");
    let dn = s.query("episodic", &Query::Dense { vector_name: "summary".into(), vector: vec![0.85, 0.05, 0.05, 0.05] }, &alice, 3, None).unwrap();
    assert_eq!(dn[0].id, "ep_mem-1");
    let bm = s.query("episodic", &Query::Bm25 { field: Some("details".into()), text: "Sarah".into() }, &alice, 3, None).unwrap();
    assert_eq!(bm[0].id, "ep_mem-2");
    // and the store still accepts writes afterwards
    s.upsert("episodic", vec![pt("ep_mem-9", "alice", "written after compaction", "x", None, 9, "work")]).unwrap();
    assert_eq!(s.count("episodic", &alice).unwrap(), 5);
    s.flush("episodic").unwrap();
    drop(s);
    // survives reopen, and the temporary rebuild dir is gone
    assert!(!d.path().join("episodic~r").exists());
    let s2 = EdgeVectorStore::open(d.path(), "english").unwrap();
    s2.ensure(&spec()).unwrap();
    assert_eq!(s2.count("episodic", &alice).unwrap(), 5);
}

#[test]
fn compact_on_a_never_written_collection_is_a_harmless_noop() {
    let dir = tempfile::tempdir().unwrap();
    let s = EdgeVectorStore::open(dir.path(), "english").unwrap();
    s.ensure(&spec()).unwrap();
    s.compact("episodic").unwrap();
    assert_eq!(s.count("episodic", &PayloadFilter::eq("user_id", "alice")).unwrap(), 0);
    s.upsert("episodic", vec![pt("ep_mem-1", "alice", "first write after empty compaction", "d", Some([1.0, 0.0, 0.0, 0.0]), 1, "work")]).unwrap();
    assert_eq!(s.count("episodic", &PayloadFilter::eq("user_id", "alice")).unwrap(), 1);
}

#[test]
fn overlong_windows_data_directory_is_rejected_with_a_clear_message() {
    if cfg!(windows) {
        let long = std::path::PathBuf::from(format!("C:/{}", "a".repeat(150)));
        let e = EdgeVectorStore::open(&long, "english").err().unwrap().to_string();
        assert!(e.contains("shorter data folder"), "{e}");
    }
}

#[test]
fn long_field_names_do_not_lengthen_paths() {
    let dir = tempfile::tempdir().unwrap();
    let s = EdgeVectorStore::open(dir.path(), "english").unwrap();
    let spec = CollectionSpec { name: "x".into(), dense: vec![], text_fields: vec!["a_really_quite_long_field_name_here".into()] };
    s.ensure(&spec).unwrap();
    let mut p = VecPoint { id: "p1".into(), ..Default::default() };
    p.payload.insert("user_id".into(), "u".into());
    p.payload.insert("a_really_quite_long_field_name_here".into(), "hello world".into());
    p.text.insert("a_really_quite_long_field_name_here".into(), "hello world".into());
    s.upsert("x", vec![p]).unwrap();
    s.flush("x").unwrap();
    fn longest(d: &std::path::Path) -> usize {
        std::fs::read_dir(d).unwrap().flatten().map(|e| { let p = e.path(); if p.is_dir() { longest(&p) } else { p.strip_prefix(d.ancestors().last().unwrap()).map(|_| p.to_string_lossy().len()).unwrap_or(0) } }).max().unwrap_or(0)
    }
    let rel_max = longest(&dir.path().join("x")) - dir.path().join("x").to_string_lossy().len();
    assert!(rel_max < 110, "relative path inside a collection stays short, got {rel_max}");
    let h = s.query("x", &Query::Bm25 { field: Some("a_really_quite_long_field_name_here".into()), text: "hello".into() }, &PayloadFilter::default(), 3, None).unwrap();
    assert_eq!(h.len(), 1);
}
