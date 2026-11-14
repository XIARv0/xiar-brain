// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Vector/search store: dense vectors + BM25 sparse text + payload, in-process (Qdrant Edge fork).
//!
//! One collection per memory layer. Each point has
//!   * zero or more named dense vectors (one per embedded field; cosine distance),
//!   * BM25 sparse vectors: `text` (all fields joined) and `text_<field>` (for field-restricted search),
//!   * a JSON payload (the item itself plus the string id under `_id`).
//!

//! String ids of any shape are mapped to deterministic UUIDv5 point ids.

use crate::{Result, StoreError};

/// A point's payload with its named dense vectors.
pub type DensePoint = (Payload, BTreeMap<String, Vec<f32>>);
use qdrant_edge::bm25_embed::{EdgeBm25, EdgeBm25Config};
use qdrant_edge::{
    Condition, Distance, EdgeConfig, EdgeShard, EdgeSparseVectorParams, EdgeVectorParams, FieldCondition, Filter, Fusion, Match, Modifier,
    NamedQuery, PointId, PointInsertOperations, PointOperations, PointStruct, PrefetchBuilder, QueryEnum, QueryRequestBuilder, Range,
    RetrieveRequestBuilder, ScoringQuery, ScrollRequestBuilder, UpdateOperation, ValueVariants, VectorInternal, Vectors, WithPayloadInterface,
    WithVector,
};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub type Payload = Map<String, Value>;

const ID_FIELD: &str = "_id";
const ALL_TEXT: &str = "t";

fn ns() -> uuid::Uuid {
    uuid::Uuid::from_bytes([0x58, 0x49, 0x41, 0x52, 0x56, 0x30, 0x4d, 0x45, 0x4d, 0x4f, 0x52, 0x59, 0x00, 0x00, 0x00, 0x01])
}

fn edge_uuid(u: uuid::Uuid) -> PointId {
    PointId::Uuid(qdrant_edge::external::uuid::Uuid::from_bytes(*u.as_bytes()))
}

fn point_id(id: &str) -> PointId {
    edge_uuid(uuid::Uuid::new_v5(&ns(), id.as_bytes()))
}

/// What a collection holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionSpec {
    pub name: String,
    /// named dense vectors and their dimensions (empty = BM25-only collection)
    pub dense: Vec<(String, u32)>,
    /// text fields: joined into the point's BM25 text; a BM25 query may be restricted to one of them
    pub text_fields: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct VecPoint {
    pub id: String,
    pub payload: Payload,
    /// vector name -> embedding (must match the collection's dimension)
    pub dense: BTreeMap<String, Vec<f32>>,
    /// text field -> text (feeds BM25)
    pub text: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    /// payload key (dotted path allowed) equals a scalar
    Eq(String, Value),
    /// payload key is one of the scalars
    Any(String, Vec<Value>),
    /// numeric range, inclusive bounds
    Range { key: String, gte: Option<f64>, lte: Option<f64> },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PayloadFilter {
    pub must: Vec<Cond>,
    pub must_not: Vec<Cond>,
}

impl PayloadFilter {
    pub fn eq(key: &str, v: impl Into<Value>) -> Self {
        Self { must: vec![Cond::Eq(key.into(), v.into())], must_not: vec![] }
    }
    pub fn and(mut self, c: Cond) -> Self {
        self.must.push(c);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FusionKind {
    Rrf,
    Dbsf,
    /// RRF with per-source weights: (dense, bm25)
    Weighted(f32, f32),
}

#[derive(Debug, Clone)]
pub enum Query {
    Dense { vector_name: String, vector: Vec<f32> },
    /// `field = None` searches the combined text
    Bm25 { field: Option<String>, text: String },
    Hybrid { vector_name: String, vector: Vec<f32>, field: Option<String>, text: String, fusion: FusionKind },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    pub score: f32,
    pub payload: Payload,
}

pub trait VectorStore: Send + Sync {
    /// Create the collection or verify an existing one is compatible.
    fn ensure(&self, spec: &CollectionSpec) -> Result<()>;
    /// Index one payload field for exact matching, so a filter on it does not read every point. Safe to call again; a store
    /// without indexes ignores it.
    fn ensure_keyword_index(&self, _collection: &str, _field: &str) -> Result<()> {
        Ok(())
    }
    fn upsert(&self, collection: &str, points: Vec<VecPoint>) -> Result<()>;
    fn get(&self, collection: &str, ids: &[String]) -> Result<Vec<Option<Payload>>>;
    /// Like `get`, but also returns each point's dense vectors (needed to keep embeddings when updating a point).
    fn get_dense(&self, collection: &str, ids: &[String]) -> Result<Vec<Option<DensePoint>>>;
    fn delete(&self, collection: &str, ids: &[String]) -> Result<()>;
    /// Delete every point matching the filter; returns how many were removed. An empty filter is refused.
    fn delete_where(&self, collection: &str, filter: &PayloadFilter) -> Result<usize>;
    fn query(&self, collection: &str, q: &Query, filter: &PayloadFilter, limit: usize, score_threshold: Option<f32>) -> Result<Vec<Hit>>;
    /// Points matching the filter (payload only), in storage order. Pass the returned opaque cursor to get the next page.
    fn scroll(&self, collection: &str, filter: &PayloadFilter, limit: usize, cursor: Option<&str>) -> Result<(Vec<Payload>, Option<String>)>;
    fn count(&self, collection: &str, filter: &PayloadFilter) -> Result<usize>;
    fn flush(&self, collection: &str) -> Result<()>;
    /// Merge/rewrite segments so deleted points are physically dropped (used after erasure).
    fn compact(&self, collection: &str) -> Result<()>;
}

struct Coll {
    shard: EdgeShard,
    spec: CollectionSpec,
    bm25: EdgeBm25,
    indexed: Mutex<std::collections::BTreeSet<String>>,
}

pub struct EdgeVectorStore {
    root: PathBuf,
    language: String,
    colls: Mutex<HashMap<String, Arc<Coll>>>,
}

fn be<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Backend(format!("vector store: {e}"))
}

/// Only one BM25 sparse vector (`t`, all text fields joined) is stored per point: a separate sparse store per field
/// costs ~2 MB of preallocated files each and deepens directory paths. Field-restricted BM25 queries run on the
/// combined text and keep the hits whose chosen field contains a query token (see `field_filter`).
fn sparse_name(spec: &CollectionSpec, field: &Option<String>) -> Result<String> {
    if let Some(f) = field {
        if !spec.text_fields.contains(f) {
            return Err(StoreError::Backend(format!("unknown text field {f:?}")));
        }
    }
    Ok(ALL_TEXT.to_string())
}

/// Cleaned query tokens (lowercase, ASCII punctuation removed, tokens of 2+ chars).
/// Does `payload[field]` share at least one BM25 term with the query? Both sides go through the collection's own tokenizer
/// (lower-casing, stop words, stemming), so the restriction agrees with what the ranking matched: "parks" reaches "park",
/// "the" matches nothing.
fn field_shares_term(bm25: &EdgeBm25, payload: &Payload, field: &str, query_terms: &[u32]) -> bool {
    let Some(text) = payload.get(field).and_then(Value::as_str) else { return false };
    let terms = bm25.embed_document(text).indices;
    terms.iter().any(|t| query_terms.contains(t))
}

/// Longest data directory we accept: leaves room for `<collection>/segments/<uuid>/vector_storage-f3/store/page_0.dat`.
const MAX_ROOT_PATH_CHARS: usize = 140;

fn value_variant(v: &Value) -> Result<ValueVariants> {
    Ok(match v {
        Value::String(s) => ValueVariants::String(s.clone()),
        Value::Bool(b) => ValueVariants::Bool(*b),
        Value::Number(n) if n.is_i64() => ValueVariants::Integer(n.as_i64().unwrap()),
        other => return Err(StoreError::Backend(format!("unsupported filter value {other}"))),
    })
}

fn key(k: &str) -> Result<qdrant_edge::JsonPath> {
    k.try_into().map_err(|_| StoreError::Backend(format!("invalid payload key {k:?}")))
}

fn range_cond(k: &str, gte: Option<f64>, lte: Option<f64>) -> Result<Condition> {
    use qdrant_edge::external::ordered_float::OrderedFloat;
    let r: Range<OrderedFloat<f64>> = Range { gte: gte.map(OrderedFloat), lte: lte.map(OrderedFloat), ..Default::default() };
    Ok(Condition::Field(FieldCondition::new_range(key(k)?, r)))
}

fn cond(c: &Cond) -> Result<Condition> {
    Ok(match c {
        Cond::Eq(k, v) => Condition::Field(FieldCondition::new_match(key(k)?, Match::new_value(value_variant(v)?))),
        Cond::Any(k, vs) => {
            let variants = if vs.iter().all(Value::is_string) {
                qdrant_edge::AnyVariants::Strings(vs.iter().map(|v| v.as_str().unwrap().into()).collect())
            } else if vs.iter().all(|v| v.is_i64()) {
                qdrant_edge::AnyVariants::Integers(vs.iter().map(|v| v.as_i64().unwrap()).collect())
            } else {
                return Err(StoreError::Backend("Any filter needs all strings or all integers".into()));
            };
            Condition::Field(FieldCondition::new_match(key(k)?, Match::new_any(variants)))
        }
        Cond::Range { key: k, gte, lte } => range_cond(k, *gte, *lte)?,
    })
}

fn to_filter(f: &PayloadFilter) -> Result<Option<Filter>> {
    if f.must.is_empty() && f.must_not.is_empty() {
        return Ok(None);
    }
    let must: Vec<Condition> = f.must.iter().map(cond).collect::<Result<_>>()?;
    let must_not: Vec<Condition> = f.must_not.iter().map(cond).collect::<Result<_>>()?;
    Ok(Some(Filter {
        must: if must.is_empty() { None } else { Some(must) },
        must_not: if must_not.is_empty() { None } else { Some(must_not) },
        should: None,
        min_should: None,
    }))
}

fn create_keyword_index(shard: &EdgeShard, field: &str) -> Result<()> {
    let op = qdrant_edge::FieldIndexOperations::CreateIndex(qdrant_edge::CreateIndex { field_name: key(field)?, field_schema: Some(qdrant_edge::PayloadFieldSchema::FieldType(qdrant_edge::PayloadSchemaType::Keyword)) });
    shard.update(UpdateOperation::FieldIndexOperation(op)).map_err(be)
}

fn payload_of(p: Option<qdrant_edge::Payload>) -> Payload {
    p.map(|p| p.0.into_iter().collect()).unwrap_or_default()
}

impl EdgeVectorStore {
    pub fn open(root: &Path, bm25_language: &str) -> Result<Self> {
        let len = root.to_string_lossy().chars().count();
        if cfg!(windows) && len > MAX_ROOT_PATH_CHARS {
            return Err(StoreError::Backend(format!("vector store directory path is {len} characters; on Windows it must be at most {MAX_ROOT_PATH_CHARS} (the engine nests directories deeply). Choose a shorter data folder.")));
        }
        std::fs::create_dir_all(root).map_err(be)?;
        Ok(Self { root: root.to_path_buf(), language: bm25_language.to_string(), colls: Mutex::new(HashMap::new()) })
    }

    fn coll(&self, name: &str) -> Result<Arc<Coll>> {
        self.colls.lock().unwrap().get(name).cloned().ok_or_else(|| StoreError::Backend(format!("collection {name:?} not initialised (call ensure)")))
    }

    fn config_for(spec: &CollectionSpec) -> EdgeConfig {
        let mut vectors = HashMap::new();
        for (n, dim) in &spec.dense {
            vectors.insert(n.clone(), EdgeVectorParams::builder(*dim as usize, Distance::Cosine).build());
        }
        let mut sparse = HashMap::new();
        sparse.insert(ALL_TEXT.to_string(), EdgeSparseVectorParams { modifier: Some(Modifier::Idf), ..Default::default() });
        // small WAL segments: upstream's 32 MiB default is preallocated per collection
        let wal = qdrant_edge::WalOptions { segment_capacity: 2 * 1024 * 1024, ..Default::default() };
        EdgeConfig { vectors, sparse_vectors: sparse, wal_options: Some(wal), ..Default::default() }
    }
}

impl VectorStore for EdgeVectorStore {
    fn ensure(&self, spec: &CollectionSpec) -> Result<()> {
        let mut map = self.colls.lock().unwrap();
        if let Some(existing) = map.get(&spec.name) {
            if existing.spec != *spec {
                return Err(StoreError::Backend(format!("collection {:?} already open with a different schema", spec.name)));
            }
            return Ok(());
        }
        let dir = self.root.join(&spec.name);
        let cfg = Self::config_for(spec);
        let shard = if dir.join("edge_config.json").exists() {
            EdgeShard::load(&dir, Some(cfg)).map_err(|e| {
                StoreError::Backend(format!(
                    "collection {:?} on disk is incompatible with the requested schema (embedding dimension changed? re-index needed): {e}",
                    spec.name
                ))
            })?
        } else {
            std::fs::create_dir_all(&dir).map_err(be)?;
            EdgeShard::new(&dir, cfg).map_err(be)?
        };
        let bm25 = EdgeBm25::new(EdgeBm25Config { language: Some(self.language.clone()), ..Default::default() }).map_err(be)?;
        map.insert(spec.name.clone(), Arc::new(Coll { shard, spec: spec.clone(), bm25, indexed: Mutex::new(Default::default()) }));
        Ok(())
    }

    fn ensure_keyword_index(&self, collection: &str, field: &str) -> Result<()> {
        let c = self.coll(collection)?;
        if !c.indexed.lock().unwrap().insert(field.to_string()) {
            return Ok(());
        }
        create_keyword_index(&c.shard, field)
    }

    fn upsert(&self, collection: &str, points: Vec<VecPoint>) -> Result<()> {
        let c = self.coll(collection)?;
        let mut out = Vec::with_capacity(points.len());
        for p in points {
            let mut named: Vec<(String, VectorInternal)> = Vec::new();
            for (name, vec) in &p.dense {
                let Some((_, dim)) = c.spec.dense.iter().find(|(n, _)| n == name) else {
                    return Err(StoreError::Backend(format!("unknown dense vector {name:?} for collection {collection:?}")));
                };
                if vec.len() != *dim as usize {
                    return Err(StoreError::DimensionMismatch { name: name.clone(), expected: *dim as usize, got: vec.len() });
                }
                named.push((name.clone(), VectorInternal::from(vec.clone())));
            }
            let all: String = p.text.values().cloned().collect::<Vec<_>>().join("\n");
            named.push((ALL_TEXT.to_string(), c.bm25.embed_document(&all).into()));
            let mut payload = p.payload.clone();
            payload.insert(ID_FIELD.into(), Value::String(p.id.clone()));
            out.push(PointStruct::new(point_id(&p.id), Vectors::new_named(named), Value::Object(payload)).into());
        }
        c.shard.update(UpdateOperation::PointOperation(PointOperations::UpsertPoints(PointInsertOperations::PointsList(out)))).map_err(be)
    }

    fn get(&self, collection: &str, ids: &[String]) -> Result<Vec<Option<Payload>>> {
        let c = self.coll(collection)?;
        let recs = c
            .shard
            .retrieve(
                RetrieveRequestBuilder::new(ids.iter().map(|i| point_id(i)).collect())
                    .with_payload(WithPayloadInterface::Bool(true))
                    .with_vector(WithVector::Bool(false))
                    .build(),
            )
            .map_err(be)?;
        let by_id: HashMap<String, Payload> = recs.into_iter().map(|r| (format!("{:?}", r.id), payload_of(r.payload))).collect();
        Ok(ids.iter().map(|i| by_id.get(&format!("{:?}", point_id(i))).cloned()).collect())
    }

    fn get_dense(&self, collection: &str, ids: &[String]) -> Result<Vec<Option<DensePoint>>> {
        let c = self.coll(collection)?;
        let recs = c
            .shard
            .retrieve(
                RetrieveRequestBuilder::new(ids.iter().map(|i| point_id(i)).collect())
                    .with_payload(WithPayloadInterface::Bool(true))
                    .with_vector(WithVector::Bool(true))
                    .build(),
            )
            .map_err(be)?;
        let mut by_id: HashMap<String, (Payload, BTreeMap<String, Vec<f32>>)> = HashMap::new();
        for r in recs {
            let mut dense = BTreeMap::new();
            if let Some(qdrant_edge::VectorStructInternal::Named(map)) = r.vector {
                for (name, v) in map {
                    if let VectorInternal::Dense(d) = v {
                        dense.insert(name, d);
                    }
                }
            }
            by_id.insert(format!("{:?}", r.id), (payload_of(r.payload), dense));
        }
        Ok(ids.iter().map(|i| by_id.get(&format!("{:?}", point_id(i))).cloned()).collect())
    }

    fn delete(&self, collection: &str, ids: &[String]) -> Result<()> {
        let c = self.coll(collection)?;
        c.shard.update(UpdateOperation::PointOperation(PointOperations::DeletePoints { ids: ids.iter().map(|i| point_id(i)).collect() })).map_err(be)
    }

    fn delete_where(&self, collection: &str, filter: &PayloadFilter) -> Result<usize> {
        let c = self.coll(collection)?;
        let Some(f) = to_filter(filter)? else { return Err(StoreError::Backend("refusing delete_where with an empty filter".into())) };
        let n = self.count(collection, filter)?;
        c.shard.update(UpdateOperation::PointOperation(PointOperations::DeletePointsByFilter(f))).map_err(be)?;
        Ok(n)
    }

    fn query(&self, collection: &str, q: &Query, filter: &PayloadFilter, limit: usize, score_threshold: Option<f32>) -> Result<Vec<Hit>> {
        let c = self.coll(collection)?;
        let dense_q = |name: &str, v: &[f32]| -> Result<QueryEnum> {
            let Some((_, dim)) = c.spec.dense.iter().find(|(n, _)| n == name) else {
                return Err(StoreError::Backend(format!("unknown dense vector {name:?}")));
            };
            if v.len() != *dim as usize {
                return Err(StoreError::DimensionMismatch { name: name.into(), expected: *dim as usize, got: v.len() });
            }
            Ok(QueryEnum::Nearest(NamedQuery { query: VectorInternal::from(v.to_vec()), using: Some(name.to_string()) }))
        };
        let sparse_q = |field: &Option<String>, text: &str| -> Result<QueryEnum> {
            let using = sparse_name(&c.spec, field)?;
            Ok(QueryEnum::Nearest(NamedQuery { query: VectorInternal::from(c.bm25.embed_query(text)), using: Some(using) }))
        };
        let f = to_filter(filter)?;
        // a field-restricted text query is filtered after retrieval, so fetch extra candidates
        let restrict: Option<(String, Vec<u32>)> = match q {
            Query::Bm25 { field: Some(f), text } | Query::Hybrid { field: Some(f), text, .. } => Some((f.clone(), c.bm25.embed_query(text).indices)),
            _ => None,
        };
        let fetch_limit = if restrict.is_some() { (limit * 5).max(50) } else { limit };
        let mut req = QueryRequestBuilder::new(fetch_limit).with_payload(WithPayloadInterface::Bool(true));
        if let Some(t) = score_threshold {
            req = req.score_threshold(t);
        }
        req = match q {
            Query::Dense { vector_name, vector } => {
                let mut r = req.query(ScoringQuery::Vector(dense_q(vector_name, vector)?));
                if let Some(f) = f {
                    r = r.filter(f);
                }
                r
            }
            Query::Bm25 { field, text } => {
                let mut r = req.query(ScoringQuery::Vector(sparse_q(field, text)?));
                if let Some(f) = f {
                    r = r.filter(f);
                }
                r
            }
            Query::Hybrid { vector_name, vector, field, text, fusion } => {
                use qdrant_edge::external::ordered_float::OrderedFloat;
                let fetch = (fetch_limit * 4).max(20);
                let mut d = PrefetchBuilder::new(fetch).query(ScoringQuery::Vector(dense_q(vector_name, vector)?));
                let mut s = PrefetchBuilder::new(fetch).query(ScoringQuery::Vector(sparse_q(field, text)?));
                if let Some(f) = &f {
                    d = d.filter(f.clone());
                    s = s.filter(f.clone());
                }
                let fus = match fusion {
                    FusionKind::Rrf => Fusion::Rrf { k: 60, weights: None },
                    FusionKind::Dbsf => Fusion::Dbsf,
                    FusionKind::Weighted(wd, ws) => Fusion::Rrf { k: 60, weights: Some(vec![OrderedFloat(*wd), OrderedFloat(*ws)]) },
                };
                req.add_prefetch(d.build()).add_prefetch(s.build()).query(ScoringQuery::Fusion(fus))
            }
        };
        let res = c.shard.query(req.build()).map_err(be)?;
        let mut hits: Vec<Hit> = res
            .into_iter()
            .map(|sp| {
                let payload = payload_of(sp.payload);
                let id = payload.get(ID_FIELD).and_then(Value::as_str).unwrap_or_default().to_string();
                Hit { id, score: sp.score, payload }
            })
            .collect();
        if let Some((field, tokens)) = &restrict {
            hits.retain(|h| field_shares_term(&c.bm25, &h.payload, field, tokens));
        }
        hits.truncate(limit);
        Ok(hits)
    }

    fn scroll(&self, collection: &str, filter: &PayloadFilter, limit: usize, cursor: Option<&str>) -> Result<(Vec<Payload>, Option<String>)> {
        let c = self.coll(collection)?;
        let mut req = ScrollRequestBuilder::new().limit(limit).with_payload(WithPayloadInterface::Bool(true)).with_vector(WithVector::Bool(false));
        if let Some(f) = to_filter(filter)? {
            req = req.filter(f);
        }
        if let Some(cur) = cursor {
            let u = uuid::Uuid::parse_str(cur).map_err(|_| StoreError::Backend("invalid scroll cursor".into()))?;
            req = req.offset(edge_uuid(u));
        }
        let (recs, next) = c.shard.scroll(req.build()).map_err(be)?;
        let payloads: Vec<Payload> = recs.into_iter().map(|r| payload_of(r.payload)).collect();
        // Qdrant's offset is inclusive: the engine returns the id of the FIRST point of the next page.
        let next = match next {
            Some(PointId::Uuid(u)) => Some(u.to_string()),
            _ => None,
        };
        Ok((payloads, next))
    }

    fn count(&self, collection: &str, filter: &PayloadFilter) -> Result<usize> {
        let c = self.coll(collection)?;
        let mut req = qdrant_edge::CountRequestBuilder::new().exact(true);
        if let Some(f) = to_filter(filter)? {
            req = req.filter(f);
        }
        c.shard.count(req.build()).map_err(be)
    }

    fn flush(&self, collection: &str) -> Result<()> {
        self.coll(collection)?.shard.flush().map_err(be)
    }

    /// Physically rebuild the collection from its live points: deleted payloads linger in Edge's page files and WAL
    /// until a rewrite, so erasure must copy the survivors into a fresh shard and delete the old directory.
    /// (Filesystem/SSD-level remnants are out of scope: use OS full-disk encryption for those.)
    fn compact(&self, collection: &str) -> Result<()> {
        let mut map = self.colls.lock().unwrap();
        let Some(old) = map.get(collection).cloned() else { return Err(StoreError::Backend(format!("collection {collection:?} not initialised (call ensure)"))) };
        old.shard.flush().map_err(be)?;
        let dir = self.root.join(collection);
        let tmp = self.root.join(format!("{collection}~r"));
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp).map_err(be)?;
        }
        std::fs::create_dir_all(&tmp).map_err(be)?;
        {
            let fresh = EdgeShard::new(&tmp, Self::config_for(&old.spec)).map_err(be)?;
            let mut cursor: Option<PointId> = None;
            loop {
                let mut req = ScrollRequestBuilder::new().limit(256).with_payload(WithPayloadInterface::Bool(true)).with_vector(WithVector::Bool(true));
                if let Some(c) = cursor.take() {
                    req = req.offset(c);
                }
                let (recs, next) = old.shard.scroll(req.build()).map_err(be)?;
                let mut batch = Vec::with_capacity(recs.len());
                for r in recs {
                    let named: Vec<(String, VectorInternal)> = match r.vector {
                        Some(qdrant_edge::VectorStructInternal::Named(m)) => m.into_iter().collect(),
                        _ => Vec::new(),
                    };
                    let payload = Value::Object(payload_of(r.payload));
                    batch.push(PointStruct::new(r.id, Vectors::new_named(named), payload).into());
                }
                if !batch.is_empty() {
                    fresh.update(UpdateOperation::PointOperation(PointOperations::UpsertPoints(PointInsertOperations::PointsList(batch)))).map_err(be)?;
                }
                match next {
                    Some(n) => cursor = Some(n),
                    None => break,
                }
            }
            fresh.flush().map_err(be)?;
        }
        // release the old shard (its files must be closed before the directory can be removed on Windows)
        map.remove(collection);
        if Arc::strong_count(&old) > 1 {
            // someone is mid-operation on the old shard; put it back and ask the caller to retry
            map.insert(collection.to_string(), old);
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(StoreError::Backend(format!("collection {collection:?} is busy; retry compaction")));
        }
        let spec = old.spec.clone();
        let indexed = old.indexed.lock().unwrap().clone();
        drop(old);
        std::fs::remove_dir_all(&dir).map_err(be)?;
        std::fs::rename(&tmp, &dir).map_err(be)?;
        let shard = EdgeShard::load(&dir, Some(Self::config_for(&spec))).map_err(be)?;
        let bm25 = EdgeBm25::new(EdgeBm25Config { language: Some(self.language.clone()), ..Default::default() }).map_err(be)?;
        // the rebuilt shard starts without its indexes
        for field in &indexed {
            create_keyword_index(&shard, field)?;
        }
        map.insert(collection.to_string(), Arc::new(Coll { shard, spec, bm25, indexed: Mutex::new(indexed) }));
        Ok(())
    }
}
