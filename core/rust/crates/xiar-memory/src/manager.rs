// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Generic manager for the vector-backed memory layers (episodic, semantic, procedural, resource, knowledge vault,
//! raw). One implementation replaces MIRIX's five near-identical `*_manager.py` files; per-layer behaviour lives in
//! the [`MemoryRecord`] impls in `records.rs`.
//!
//! Every operation is scoped to a `user_id`: a manager can never read, modify or delete another user's items, even if
//! handed their ids (ownership is checked against the stored payload).

use crate::filter_tags::{self, FilterError, Tags};
use crate::search::{Fusion, SearchMethod, SearchRequest};
use crate::types::{EmbeddingMeta, MemoryKind, Ms, ValidationError};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};
use xiar_store::vector::{CollectionSpec, Cond, FusionKind, PayloadFilter, Query, VecPoint, VectorStore};
use xiar_store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum MemError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Filter(#[from] FilterError),
    #[error("{0} not found")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("{0}")]
    Invalid(String),
    #[error("serialization error: {0}")]
    Serde(String),
}

pub type Result<T> = std::result::Result<T, MemError>;

/// Seals/opens secrets with the owning user's key (implemented for `xiar_store::keyring::Keyring`).
pub trait Sealer: Send + Sync {
    fn seal(&self, user: &str, aad: &[u8], plaintext: &[u8]) -> std::result::Result<Vec<u8>, StoreError>;
    fn open(&self, user: &str, aad: &[u8], sealed: &[u8]) -> std::result::Result<Vec<u8>, StoreError>;
}

impl Sealer for xiar_store::keyring::Keyring {
    fn seal(&self, user: &str, aad: &[u8], plaintext: &[u8]) -> std::result::Result<Vec<u8>, StoreError> {
        xiar_store::keyring::Keyring::seal(self, user, aad, plaintext)
    }
    fn open(&self, user: &str, aad: &[u8], sealed: &[u8]) -> std::result::Result<Vec<u8>, StoreError> {
        xiar_store::keyring::Keyring::open(self, user, aad, sealed)
    }
}

/// What a memory layer must provide to be managed generically.
pub trait MemoryRecord: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    const KIND: MemoryKind;
    fn id(&self) -> &str;
    fn user_id(&self) -> &str;
    fn client_id(&self) -> Option<&str>;
    fn filter_tags(&self) -> Option<&Tags>;
    /// when the item was last modified (MIRIX `last_modify.timestamp`): "most recently updated"
    fn modified_ms(&self) -> Ms;
    /// default listing order for an empty query: `created_at` (episodic overrides with `occurred_at`)
    fn listing_ms(&self) -> Ms;
    /// event time used by time-range search (episodic/raw)
    fn occurred_ms(&self) -> Option<Ms> {
        None
    }
    fn validate(&self) -> std::result::Result<(), ValidationError>;
    fn set_embedding(&mut self, meta: Option<EmbeddingMeta>);

    /// Item -> stored payload. Layers with secrets override this to seal them.
    fn to_payload(&self, _sealer: &dyn Sealer) -> Result<Map<String, Value>> {
        match serde_json::to_value(self).map_err(|e| MemError::Serde(e.to_string()))? {
            Value::Object(m) => Ok(m),
            _ => Err(MemError::Serde("record did not serialize to an object".into())),
        }
    }
    fn from_payload(mut payload: Map<String, Value>, _sealer: &dyn Sealer) -> Result<Self> {
        payload.remove("_id");
        serde_json::from_value(Value::Object(payload)).map_err(|e| MemError::Serde(e.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scored<T> {
    pub item: T,
    pub score: f32,
}

pub struct MemoryManager<T: MemoryRecord> {
    store: Arc<dyn VectorStore>,
    sealer: Arc<dyn Sealer>,
    dense: Vec<(String, u32)>,
    /// serializes read-modify-write updates (single-process app)
    write_lock: Mutex<()>,
    _t: PhantomData<fn() -> T>,
}

const MAX_SCAN: usize = 50_000;

impl<T: MemoryRecord> MemoryManager<T> {
    /// `dims`: embedding dimension per embedded field, e.g. `[("summary", 768)]`. Empty = text-only (BM25) mode.
    pub fn new(store: Arc<dyn VectorStore>, sealer: Arc<dyn Sealer>, dims: &[(&str, u32)]) -> Result<Self> {
        let valid = T::KIND.vector_fields();
        let mut dense = Vec::new();
        for (name, dim) in dims {
            if !valid.contains(name) {
                return Err(MemError::Invalid(format!("{:?} has no embedded field {name:?} (valid: {valid:?})", T::KIND)));
            }
            dense.push((name.to_string(), *dim));
        }
        store.ensure(&CollectionSpec {
            name: T::KIND.collection().to_string(),
            dense: dense.clone(),
            text_fields: T::KIND.text_fields().iter().map(|s| s.to_string()).collect(),
        })?;
        Ok(Self { store, sealer, dense, write_lock: Mutex::new(()), _t: PhantomData })
    }

    fn coll(&self) -> &'static str {
        T::KIND.collection()
    }

    fn user_filter(user: &str) -> PayloadFilter {
        PayloadFilter::eq("user_id", user)
    }

    fn texts(&self, payload: &Map<String, Value>) -> BTreeMap<String, String> {
        // Strings and string-arrays (e.g. semantic entity_types) are indexed for BM25.
        // Arrays are space-joined so class-level queries can match typed memories.
        T::KIND
            .text_fields()
            .iter()
            .filter_map(|f| match payload.get(*f) {
                Some(Value::String(s)) if !s.is_empty() => Some((f.to_string(), s.clone())),
                Some(Value::Array(arr)) => {
                    let joined = arr
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if joined.is_empty() {
                        None
                    } else {
                        Some((f.to_string(), joined))
                    }
                }
                _ => None,
            })
            .collect()
    }

    fn check_embeddings(&self, emb: &BTreeMap<String, Vec<f32>>) -> Result<()> {
        for (name, v) in emb {
            match self.dense.iter().find(|(n, _)| n == name) {
                None => return Err(MemError::Invalid(format!("embedding for {name:?} but the collection has no such vector (configured: {:?})", self.dense))),
                Some((_, dim)) if v.len() != *dim as usize => return Err(StoreError::DimensionMismatch { name: name.clone(), expected: *dim as usize, got: v.len() }.into()),
                _ => {}
            }
        }
        Ok(())
    }

    fn write(&self, item: &T, dense: BTreeMap<String, Vec<f32>>) -> Result<()> {
        let payload = item.to_payload(self.sealer.as_ref())?;
        let text = self.texts(&payload);
        self.store.upsert(self.coll(), vec![VecPoint { id: item.id().to_string(), payload, dense, text }])?;
        Ok(())
    }

    /// Insert (or replace, if the id already exists) an item with optional embeddings.
    pub fn insert(&self, item: T, embeddings: BTreeMap<String, Vec<f32>>) -> Result<T> {
        item.validate()?;
        self.check_embeddings(&embeddings)?;
        self.write(&item, embeddings)?;
        Ok(item)
    }

    fn load(&self, payload: Map<String, Value>) -> Result<T> {
        T::from_payload(payload, self.sealer.as_ref())
    }

    /// Fetch one item. `None` if it does not exist, belongs to another user, or is outside the caller's scopes.
    pub fn get(&self, user: &str, id: &str, scopes: Option<&[String]>) -> Result<Option<T>> {
        let Some(Some(p)) = self.store.get(self.coll(), &[id.to_string()])?.into_iter().next() else { return Ok(None) };
        if p.get("user_id").and_then(Value::as_str) != Some(user) {
            return Ok(None);
        }
        let item = self.load(p)?;
        Ok(if filter_tags::matches(item.filter_tags(), None, scopes)? { Some(item) } else { None })
    }

    /// Read-modify-write. `f` mutates the item and returns the names of changed fields. Existing embeddings are
    /// kept; pass `new_embeddings` for fields whose text changed so they get re-embedded.
    pub fn update<F>(&self, user: &str, id: &str, new_embeddings: BTreeMap<String, Vec<f32>>, f: F) -> Result<Option<(T, Vec<&'static str>)>>
    where
        F: FnOnce(&mut T) -> Vec<&'static str>,
    {
        self.check_embeddings(&new_embeddings)?;
        let _guard = self.write_lock.lock().unwrap();
        let Some(Some((p, mut dense))) = self.store.get_dense(self.coll(), &[id.to_string()])?.into_iter().next() else { return Ok(None) };
        if p.get("user_id").and_then(Value::as_str) != Some(user) {
            return Ok(None);
        }
        let mut item = self.load(p)?;
        let changed = f(&mut item);
        if changed.is_empty() {
            return Ok(Some((item, changed)));
        }
        item.validate()?;
        dense.extend(new_embeddings);
        self.write(&item, dense)?;
        Ok(Some((item, changed)))
    }

    pub fn delete(&self, user: &str, id: &str) -> Result<bool> {
        if self.get(user, id, None)?.is_none() {
            return Ok(false);
        }
        self.store.delete(self.coll(), &[id.to_string()])?;
        Ok(true)
    }

    /// Delete everything a user owns in this layer (erasure). Returns how many items were removed.
    pub fn delete_by_user(&self, user: &str) -> Result<usize> {
        Ok(self.store.delete_where(self.coll(), &Self::user_filter(user))?)
    }

    /// Delete everything created by a client application (all users).
    pub fn delete_by_client(&self, client_id: &str) -> Result<usize> {
        Ok(self.store.delete_where(self.coll(), &PayloadFilter::eq("client_id", client_id))?)
    }

    /// Delete every item (all users) whose `updated_at_ms` is strictly before `cutoff_ms`. Used by the raw-memory TTL job.
    pub fn purge_updated_before(&self, cutoff_ms: Ms) -> Result<usize> {
        let f = PayloadFilter { must: vec![Cond::Range { key: "updated_at_ms".into(), gte: None, lte: Some((cutoff_ms - 1) as f64) }], must_not: vec![] };
        Ok(self.store.delete_where(self.coll(), &f)?)
    }

    pub fn count(&self, user: &str) -> Result<usize> {
        Ok(self.store.count(self.coll(), &Self::user_filter(user))?)
    }

    /// Items of `user` whose payload field `key` (a value or a list of values) holds any of `values`, newest first, at most
    /// `limit` (0 = all). The store filters, so only the matches are loaded, not the user's whole memory.
    pub fn list_with_any(&self, user: &str, key: &str, values: &[String], limit: usize, scopes: Option<&[String]>, tags: Option<&Tags>) -> Result<Vec<T>> {
        if values.is_empty() || matches!(scopes, Some(s) if s.is_empty()) {
            return Ok(vec![]);
        }
        if let Some(t) = tags {
            filter_tags::validate(t)?;
        }
        self.store.ensure_keyword_index(self.coll(), key)?;
        let mut items = Vec::new();
        for it in self.scan_user(user, vec![Cond::Any(key.to_string(), values.iter().map(|v| Value::String(v.clone())).collect())])? {
            if self.visible(&it, tags, scopes)? {
                items.push(it);
            }
        }
        items.sort_by(|a, b| b.listing_ms().cmp(&a.listing_ms()).then_with(|| a.id().cmp(b.id())));
        if limit > 0 {
            items.truncate(limit);
        }
        Ok(items)
    }

    fn scan_user(&self, user: &str, extra: Vec<Cond>) -> Result<Vec<T>> {
        let mut f = Self::user_filter(user);
        f.must.extend(extra);
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let (page, next) = self.store.scroll(self.coll(), &f, 256, cursor.as_deref())?;
            for p in page {
                out.push(self.load(p)?);
            }
            if out.len() > MAX_SCAN {
                return Err(MemError::Invalid(format!("more than {MAX_SCAN} items to scan; use a narrower search")));
            }
            match next {
                Some(n) => cursor = Some(n),
                None => break,
            }
        }
        Ok(out)
    }

    fn visible(&self, item: &T, req_tags: Option<&Tags>, scopes: Option<&[String]>) -> Result<bool> {
        Ok(filter_tags::matches(item.filter_tags(), req_tags, scopes)?)
    }

    /// Default listing order (episodic: newest `occurred_at`; others: newest `created_at`). `limit = 0` = no limit.
    pub fn list_recent(&self, user: &str, limit: usize, scopes: Option<&[String]>, tags: Option<&Tags>) -> Result<Vec<T>> {
        if let Some(t) = tags {
            filter_tags::validate(t)?;
        }
        if matches!(scopes, Some(s) if s.is_empty()) {
            return Ok(vec![]);
        }
        let mut items: Vec<T> = Vec::new();
        for it in self.scan_user(user, vec![])? {
            if self.visible(&it, tags, scopes)? {
                items.push(it);
            }
        }
        items.sort_by(|a, b| b.listing_ms().cmp(&a.listing_ms()).then_with(|| a.id().cmp(b.id())));
        if limit > 0 {
            items.truncate(limit);
        }
        Ok(items)
    }

    /// The item with the newest `last_modify` timestamp (MIRIX `get_most_recently_updated_*`).
    pub fn most_recently_updated(&self, user: &str) -> Result<Option<T>> {
        Ok(self.list_recent(user, 0, None, None)?.into_iter().max_by(|a, b| a.modified_ms().cmp(&b.modified_ms()).then_with(|| b.id().cmp(a.id()))))
    }

    /// Search with any MIRIX method. Results never include other users' items; `scopes`/`filter_tags` are enforced.
    pub fn search(&self, req: &SearchRequest) -> Result<Vec<Scored<T>>> {
        if let Some(t) = &req.filter_tags {
            filter_tags::validate(t)?;
        }
        if matches!(&req.scopes, Some(s) if s.is_empty()) || req.limit == 0 {
            return Ok(vec![]);
        }
        let field = req.field.as_str();
        if !field.is_empty() && !T::KIND.text_fields().contains(&field) && !T::KIND.vector_fields().contains(&field) {
            return Err(MemError::Invalid(format!("unknown search field {field:?} for {:?}", T::KIND)));
        }
        if req.query.trim().is_empty() && req.method != SearchMethod::Embedding {
            // MIRIX returns the most recent items for an empty text query
            return Ok(self
                .list_recent(&req.user_id, 0, req.scopes.as_deref(), req.filter_tags.as_ref())?
                .into_iter()
                .filter(|i| time_ok(i, req))
                .take(req.limit)
                .map(|item| Scored { item, score: 0.0 })
                .collect());
        }
        match req.method {
            SearchMethod::StringMatch | SearchMethod::FuzzyMatch => self.local_text_search(req),
            _ => self.indexed_search(req),
        }
    }

    fn base_filter(&self, req: &SearchRequest) -> PayloadFilter {
        let mut f = Self::user_filter(&req.user_id);
        if req.start_ms.is_some() || req.end_ms.is_some() {
            f.must.push(Cond::Range { key: "occurred_at_ms".into(), gte: req.start_ms.map(|v| v as f64), lte: req.end_ms.map(|v| v as f64) });
        }
        if let Some(scopes) = &req.scopes {
            f.must.push(Cond::Any("filter_tags.scope".into(), scopes.iter().map(|s| Value::String(s.clone())).collect()));
        }
        f
    }

    fn dense_field(&self, req: &SearchRequest) -> Result<String> {
        let name = if req.field.is_empty() { self.dense.first().map(|(n, _)| n.clone()) } else { Some(req.field.clone()) };
        match name {
            Some(n) if self.dense.iter().any(|(d, _)| *d == n) => Ok(n),
            Some(n) => Err(MemError::Invalid(format!("field {n:?} is not embedded in this collection"))),
            None => Err(MemError::Invalid("this collection has no embeddings configured; use bm25".into())),
        }
    }

    fn indexed_search(&self, req: &SearchRequest) -> Result<Vec<Scored<T>>> {
        let bm25_field = if req.field.is_empty() || T::KIND.vector_fields().contains(&req.field.as_str()) && !T::KIND.text_fields().contains(&req.field.as_str()) { None } else { Some(req.field.clone()) };
        let query = match (req.method, &req.embedding) {
            (SearchMethod::Embedding, Some(e)) => Query::Dense { vector_name: self.dense_field(req)?, vector: e.clone() },
            (SearchMethod::Embedding, None) => return Err(MemError::Invalid("embedding search needs a query embedding".into())),
            (SearchMethod::Hybrid, Some(e)) => Query::Hybrid {
                vector_name: self.dense_field(req)?,
                vector: e.clone(),
                field: bm25_field,
                text: req.query.clone(),
                fusion: match req.fusion {
                    Fusion::Rrf => FusionKind::Rrf,
                    Fusion::Dbsf => FusionKind::Dbsf,
                    Fusion::Weighted(d, s) => FusionKind::Weighted(d, s),
                },
            },
            // hybrid without an embedding degrades to BM25 instead of failing
            _ => Query::Bm25 { field: bm25_field, text: req.query.clone() },
        };
        let filter = self.base_filter(req);
        let threshold = if matches!(query, Query::Dense { .. }) { req.similarity_threshold } else { None };
        // over-fetch, then post-filter filter_tags; widen while the page is full but under-delivers
        let mut fetch = (req.limit * 3).max(20);
        loop {
            let hits = self.store.query(self.coll(), &query, &filter, fetch, threshold)?;
            let returned = hits.len();
            let mut out = Vec::new();
            for h in hits {
                let item = self.load(h.payload)?;
                if self.visible(&item, req.filter_tags.as_ref(), req.scopes.as_deref())? {
                    out.push(Scored { item, score: h.score });
                }
            }
            if out.len() >= req.limit || returned < fetch || fetch >= 1000 {
                out.truncate(req.limit);
                return Ok(out);
            }
            fetch *= 2;
        }
    }

    fn local_text_search(&self, req: &SearchRequest) -> Result<Vec<Scored<T>>> {
        let fuzzy = req.method == SearchMethod::FuzzyMatch;
        let needle = crate::search::clean_text(&req.query);
        let fields: Vec<&str> = if req.field.is_empty() { T::KIND.text_fields().to_vec() } else { vec![req.field.as_str()] };
        let mut extra = vec![];
        if req.start_ms.is_some() || req.end_ms.is_some() {
            extra.push(Cond::Range { key: "occurred_at_ms".into(), gte: req.start_ms.map(|v| v as f64), lte: req.end_ms.map(|v| v as f64) });
        }
        let mut out = Vec::new();
        for item in self.scan_user(&req.user_id, extra)? {
            if !self.visible(&item, req.filter_tags.as_ref(), req.scopes.as_deref())? {
                continue;
            }
            let payload = item.to_payload(self.sealer.as_ref())?;
            let mut best = 0.0f32;
            for f in &fields {
                if let Some(text) = payload.get(*f).and_then(Value::as_str) {
                    let hay = crate::search::clean_text(text);
                    let s = if hay.contains(&needle) { 1.0 } else if fuzzy { fuzzy_score(&needle, &hay) } else { 0.0 };
                    best = best.max(s);
                }
            }
            if best > 0.0 && (!fuzzy || best >= 0.8) {
                out.push(Scored { item, score: best });
            }
        }
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| b.item.listing_ms().cmp(&a.item.listing_ms())));
        out.truncate(req.limit);
        Ok(out)
    }
}

fn time_ok<T: MemoryRecord>(item: &T, req: &SearchRequest) -> bool {
    match item.occurred_ms() {
        Some(t) => req.start_ms.is_none_or(|s| t >= s) && req.end_ms.is_none_or(|e| t <= e),
        None => req.start_ms.is_none() && req.end_ms.is_none(),
    }
}

/// Typo-tolerant containment: best normalised similarity between the needle and any equally long word window.
pub fn fuzzy_score(needle: &str, hay: &str) -> f32 {
    let n_words = needle.split(' ').count().max(1);
    let words: Vec<&str> = hay.split(' ').collect();
    if words.is_empty() || needle.is_empty() {
        return 0.0;
    }
    let mut best = 0.0f32;
    for w in words.windows(n_words.min(words.len())) {
        let cand = w.join(" ");
        let d = levenshtein(needle, &cand);
        let m = needle.chars().count().max(cand.chars().count()).max(1);
        best = best.max(1.0 - d as f32 / m as f32);
    }
    best
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}
