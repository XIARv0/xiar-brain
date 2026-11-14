// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Multi-tenant MIRIX Semantic memory: one business = one physically separate store.
//!
//! Enforcement, outermost to innermost (each layer alone refuses cross-business access):
//! 1. [`BusinessId`] is mandatory: there is no tenant handle without one (`TenantRegistry::open(None)` errors).
//! 2. DB level: every business gets its own vector-store root `<root>/tenants/<slug>/`, bound by a
//!    `.xiar-tenant` marker written on creation and verified on every open. A handle for business A
//!    physically cannot address business B's collections; a renamed/swapped directory is refused.
//! 3. Write stamp: inserts are stamped `scope = business:<slug>`; an item already tagged for another
//!    business (or any other non-core scope) is refused.
//! 4. Read filter: searches run with `scopes = [business:<slug>]` and results are re-checked
//!    (defence in depth) so a mis-stamped row can never leave the tenant.
//!
//! Extends [`crate::business_ns`] (same slug rules and scope strings).

use crate::business_ns::{self, BUSINESS_PREFIX};
use crate::manager::{MemError, MemoryManager, Result, Scored, Sealer};
use crate::ontology_retrieve::search_semantic_ontology;
use crate::query_expand::QueryExpander;
use crate::types::SemanticItem;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use xiar_store::vector::EdgeVectorStore;

pub const TENANT_MARKER: &str = ".xiar-tenant";
pub const TENANTS_DIR: &str = "tenants";

/// A validated business id (normalised slug). The only way to reach tenant data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BusinessId(String);

impl BusinessId {
    pub fn parse(raw: &str) -> Result<Self> {
        let scope = business_ns::business_scope(raw).map_err(MemError::Invalid)?;
        Ok(Self(scope[BUSINESS_PREFIX.len()..].to_string()))
    }
    /// `None` / blank is refused: every read and write must name its business.
    pub fn require(raw: Option<&str>) -> Result<Self> {
        match raw {
            Some(s) if !s.trim().is_empty() => Self::parse(s),
            _ => Err(MemError::Invalid("business id is required".into())),
        }
    }
    pub fn slug(&self) -> &str {
        &self.0
    }
    pub fn scope(&self) -> String {
        format!("{BUSINESS_PREFIX}{}", self.0)
    }
}

impl std::fmt::Display for BusinessId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn item_scope(item: &SemanticItem) -> Option<String> {
    item.common.filter_tags.as_ref().and_then(|t| t.get("scope")).and_then(Value::as_str).map(str::to_string)
}

/// Semantic memory of exactly one business.
pub struct TenantSemantic {
    business: BusinessId,
    user: String,
    mgr: MemoryManager<SemanticItem>,
}

impl TenantSemantic {
    pub fn business(&self) -> &BusinessId {
        &self.business
    }

    fn scopes(&self) -> Vec<String> {
        vec![self.business.scope()]
    }

    fn owns(&self, item: &SemanticItem) -> bool {
        item_scope(item).as_deref() == Some(self.business.scope().as_str())
    }

    /// Insert, stamping this business's scope. Refuses items tagged for any other scope.
    pub fn insert(&self, mut item: SemanticItem, embeddings: BTreeMap<String, Vec<f32>>) -> Result<SemanticItem> {
        let mine = self.business.scope();
        if let Some(s) = item_scope(&item) {
            if s != mine {
                return Err(MemError::Invalid(format!("cross-business write refused: item scope {s:?}, tenant {mine:?}")));
            }
        }
        item.common.user_id = self.user.clone();
        let mut tags = item.common.filter_tags.take().unwrap_or_default();
        tags.insert("scope".into(), json!(mine));
        let mut list: Vec<Value> = tags.get("tags").and_then(Value::as_array).cloned().unwrap_or_default();
        for t in [json!(mine), json!("business")] {
            if !list.contains(&t) {
                list.push(t);
            }
        }
        tags.insert("tags".into(), Value::Array(list));
        item.common.filter_tags = Some(tags);
        self.mgr.insert(item, embeddings)
    }

    pub fn get(&self, id: &str) -> Result<Option<SemanticItem>> {
        Ok(self.mgr.get(&self.user, id, Some(&self.scopes()))?.filter(|i| self.owns(i)))
    }

    /// BM25 (+ ontology expansion when `expander` is given), scoped to this business only.
    pub fn search(&self, query: &str, limit: usize, expander: Option<&dyn QueryExpander>) -> Result<Vec<Scored<SemanticItem>>> {
        let scopes = self.scopes();
        let hits = search_semantic_ontology(&self.mgr, &self.user, query, limit, Some(&scopes), expander)?;
        Ok(hits.into_iter().filter(|h| self.owns(&h.item)).collect())
    }

    pub fn list_recent(&self, limit: usize) -> Result<Vec<SemanticItem>> {
        Ok(self.mgr.list_recent(&self.user, limit, Some(&self.scopes()), None)?.into_iter().filter(|i|self.owns(i)).collect())
    }
    pub fn delete(&self, id: &str) -> Result<bool> {
        if self.get(id)?.is_none() { return Ok(false); }
        self.mgr.delete(&self.user, id)
    }
    pub fn count(&self) -> Result<usize> {
        Ok(self.list_recent(0)?.len())
    }
}

/// Opens (and caches) per-business stores under `<root>/tenants/<slug>/`.
pub struct TenantRegistry {
    root: PathBuf,
    language: String,
    user: String,
    sealer: Arc<dyn Sealer>,
    dims: Vec<(String, u32)>,
    open: Mutex<HashMap<BusinessId, Arc<TenantSemantic>>>,
}

impl TenantRegistry {
    pub fn new(root: impl Into<PathBuf>, language: &str, user: &str, sealer: Arc<dyn Sealer>, dims: &[(&str, u32)]) -> Self {
        Self {
            root: root.into(),
            language: language.into(),
            user: user.into(),
            sealer,
            dims: dims.iter().map(|(n, d)| (n.to_string(), *d)).collect(),
            open: Mutex::new(HashMap::new()),
        }
    }

    pub fn tenant_dir(&self, b: &BusinessId) -> PathBuf {
        self.root.join(TENANTS_DIR).join(b.slug())
    }

    fn bind(dir: &Path, b: &BusinessId) -> Result<()> {
        let marker = dir.join(TENANT_MARKER);
        let io = |e: std::io::Error| MemError::Invalid(format!("tenant store {}: {e}", dir.display()));
        if marker.exists() {
            let owner = std::fs::read_to_string(&marker).map_err(io)?;
            if owner.trim() != b.scope() {
                return Err(MemError::Invalid(format!(
                    "tenant store {} belongs to {:?}, refused for {:?}",
                    dir.display(),
                    owner.trim(),
                    b.scope()
                )));
            }
            return Ok(());
        }
        if dir.exists() && std::fs::read_dir(dir).map_err(io)?.next().is_some() {
            return Err(MemError::Invalid(format!("tenant store {} has data but no owner marker; refused", dir.display())));
        }
        std::fs::create_dir_all(dir).map_err(io)?;
        std::fs::write(&marker, b.scope()).map_err(io)
    }

    /// The only entry point to business data. `None`/blank business id is an error.
    pub fn open(&self, business: Option<&str>) -> Result<Arc<TenantSemantic>> {
        let b = BusinessId::require(business)?;
        let mut open = self.open.lock().map_err(|_| MemError::Invalid("tenant registry poisoned".into()))?;
        if let Some(t) = open.get(&b) {
            return Ok(t.clone());
        }
        let dir = self.tenant_dir(&b);
        Self::bind(&dir, &b)?;
        let store = Arc::new(EdgeVectorStore::open(&dir, &self.language)?);
        let dims: Vec<(&str, u32)> = self.dims.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        let mgr = MemoryManager::new(store, self.sealer.clone(), &dims)?;
        let t = Arc::new(TenantSemantic { business: b.clone(), user: self.user.clone(), mgr });
        open.insert(b, t.clone());
        Ok(t)
    }

    /// Business slugs with a bound store on disk.
    pub fn list(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.root.join(TENANTS_DIR))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join(TENANT_MARKER).exists())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::business_ns::business_tags;
    use crate::query_expand::QueryExpander;
    use xiar_store::keyring::{Keyring, MasterKey};
    use xiar_store::relational::RedbStore;

    fn reg(dir: &Path) -> TenantRegistry {
        let ring = Arc::new(Keyring::new(Arc::new(RedbStore::in_memory().unwrap()), MasterKey::generate()));
        TenantRegistry::new(dir, "english", "owner", ring, &[])
    }

    fn fact(name: &str, summary: &str) -> SemanticItem {
        SemanticItem::new("owner", name, summary, summary, "test")
    }

    #[test]
    fn business_id_is_required() {
        let d = tempfile::tempdir().unwrap();
        let r = reg(d.path());
        assert!(r.open(None).is_err());
        assert!(r.open(Some("  ")).is_err());
        assert!(r.open(Some("../tenant-b")).is_err());
        assert_eq!(r.open(Some(" tenant-a ")).unwrap().business().slug(), "tenant-a");
    }

    #[test]
    fn cross_business_reads_return_nothing() {
        let d = tempfile::tempdir().unwrap();
        let r = reg(d.path());
        let tenant_a = r.open(Some("tenant-a")).unwrap();
        let tenant_b = r.open(Some("tenant-b")).unwrap();
        let h = tenant_a.insert(fact("Sensor Supply Co", "tenant-a sensor vendor secret pricing"), BTreeMap::new()).unwrap();
        tenant_b.insert(fact("Contoso", "tenant-b logistics vendor"), BTreeMap::new()).unwrap();
        assert_eq!(tenant_a.search("vendor", 10, None).unwrap().len(), 1);
        assert!(tenant_b.search("sensor pricing", 10, None).unwrap().is_empty());
        assert!(tenant_b.get(&h.common.id).unwrap().is_none(), "knowing the id is not enough");
        assert_eq!(tenant_a.get(&h.common.id).unwrap().unwrap().name, "Sensor Supply Co");
        assert_ne!(r.tenant_dir(tenant_a.business()), r.tenant_dir(tenant_b.business()));
    }

    #[test]
    fn cross_business_write_refused_and_stamped() {
        let d = tempfile::tempdir().unwrap();
        let r = reg(d.path());
        let tenant_a = r.open(Some("tenant-a")).unwrap();
        let mut f = fact("X", "y");
        f.common.filter_tags = Some(business_tags("tenant-b").unwrap());
        assert!(tenant_a.insert(f, BTreeMap::new()).is_err());
        let ok = tenant_a.insert(fact("A", "b"), BTreeMap::new()).unwrap();
        assert_eq!(item_scope(&ok).as_deref(), Some("business:tenant-a"));
    }

    #[test]
    fn db_refuses_swapped_tenant_directory() {
        let d = tempfile::tempdir().unwrap();
        {
            let r = reg(d.path());
            r.open(Some("tenant-a")).unwrap().insert(fact("A", "tenant-a data"), BTreeMap::new()).unwrap();
            r.open(Some("tenant-b")).unwrap();
        }
        let t = d.path().join(TENANTS_DIR);
        std::fs::rename(t.join("tenant-b"), t.join("tmp")).unwrap();
        std::fs::rename(t.join("tenant-a"), t.join("tenant-b")).unwrap();
        let r = reg(d.path());
        let err = r.open(Some("tenant-b")).err().expect("swapped store must be refused");
        assert!(err.to_string().contains("belongs to"));
        std::fs::create_dir_all(t.join("orphan")).unwrap();
        std::fs::write(t.join("orphan").join("data.bin"), b"x").unwrap();
        assert!(r.open(Some("orphan")).is_err(), "unbound data dir refused");
        assert_eq!(r.list(), vec!["tenant-b".to_string(), "tmp".to_string()]);
    }

    struct Vendors;
    impl QueryExpander for Vendors {
        fn expand(&self, _q: &str) -> Vec<String> {
            vec!["Vendor".into()]
        }
    }

    #[test]
    fn ontology_expansion_stays_inside_tenant() {
        let d = tempfile::tempdir().unwrap();
        let r = reg(d.path());
        let tenant_a = r.open(Some("tenant-a")).unwrap();
        let tenant_b = r.open(Some("tenant-b")).unwrap();
        let mut a = fact("Sensor Supply Co", "makes sensors");
        a.entity_types = vec!["Vendor".into()];
        tenant_a.insert(a, BTreeMap::new()).unwrap();
        let mut b = fact("Contoso", "moves boxes");
        b.entity_types = vec!["Vendor".into()];
        tenant_b.insert(b, BTreeMap::new()).unwrap();
        let hits = tenant_a.search("which suppliers do we use", 10, Some(&Vendors)).unwrap();
        assert_eq!(hits.iter().map(|h| h.item.name.as_str()).collect::<Vec<_>>(), vec!["Sensor Supply Co"]);
    }
}
