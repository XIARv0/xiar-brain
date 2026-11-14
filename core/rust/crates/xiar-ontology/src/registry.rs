// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The versioned ontology registry. Every version keeps its full text (OWL functional syntax), a SHA-256 of that text, who
//! created it, a status (draft, validated, promoted, deprecated), a diff summary and the last consistency result.
//! Versions are numbered v1, v2, ...; numbers are never reused. Port of the legacy SQLite registry onto the relational store.

use crate::consistency::now_ms;
use crate::models::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use xiar_owl::functional::{parse, write, ParseError};
use xiar_owl::model::Ontology;
use xiar_store::relational::RelationalStore;
use xiar_store::StoreError;

const TABLE: &str = "ontology_versions";

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("stored ontology could not be read: {0}")]
    Corrupt(String),
    #[error("not a valid ontology: {0}")]
    Invalid(#[from] ParseError),
    #[error("{0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, RegistryError>;

#[derive(Serialize, Deserialize)]
struct Record {
    info: VersionInfo,
    text: String,
    consistency: Option<ConsistencyResult>,
}

pub struct Registry {
    store: Arc<dyn RelationalStore>,
    /// numbering and read-modify-write sequences run one at a time
    lock: Mutex<()>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn number(version: &str) -> Option<u64> {
    version.strip_prefix('v')?.parse().ok()
}

impl Registry {
    pub fn new(store: Arc<dyn RelationalStore>) -> Self {
        Self { store, lock: Mutex::new(()) }
    }

    fn all(&self) -> Result<Vec<Record>> {
        let rows = self.store.scan(TABLE, b"", usize::MAX)?;
        rows.into_iter().map(|(_, v)| serde_json::from_slice(&v).map_err(|e| RegistryError::Corrupt(e.to_string()))).collect()
    }

    fn load(&self, version: &str) -> Result<Option<Record>> {
        match self.store.get(TABLE, version.as_bytes())? {
            None => Ok(None),
            Some(v) => Ok(Some(serde_json::from_slice(&v).map_err(|e| RegistryError::Corrupt(e.to_string()))?)),
        }
    }

    fn save(&self, r: &Record) -> Result<()> {
        let bytes = serde_json::to_vec(r).map_err(|e| RegistryError::Corrupt(e.to_string()))?;
        self.store.put(TABLE, r.info.version.as_bytes(), &bytes)?;
        Ok(())
    }

    /// Store `ontology` as the next version.
    pub fn register(&self, ontology: &Ontology, promoted_by: &str, migration_required: bool, diff_summary: Option<String>, status: Status, consistency: Option<ConsistencyResult>) -> Result<VersionInfo> {
        let _g = self.lock.lock().unwrap();
        let next = self.all()?.iter().filter_map(|r| number(&r.info.version)).max().unwrap_or(0) + 1;
        let text = write(ontology);
        let info = VersionInfo { version: format!("v{next}"), timestamp_ms: now_ms(), promoted_by: promoted_by.into(), migration_required, diff_summary, status, file_hash: hex(&Sha256::digest(text.as_bytes())) };
        self.save(&Record { info: info.clone(), text, consistency })?;
        Ok(info)
    }

    /// Register ontology text written by hand or exported elsewhere. The text must parse inside the XIAR profile.
    pub fn register_text(&self, text: &str, promoted_by: &str, migration_required: bool, diff_summary: Option<String>, status: Status) -> Result<VersionInfo> {
        let o = parse(text)?;
        self.register(&o, promoted_by, migration_required, diff_summary, status, None)
    }

    pub fn get(&self, version: &str) -> Result<Option<VersionInfo>> {
        Ok(self.load(version)?.map(|r| r.info))
    }

    /// Newest version, optionally only those with `status`.
    pub fn latest(&self, status: Option<Status>) -> Result<Option<VersionInfo>> {
        Ok(self.all()?.into_iter().map(|r| r.info).filter(|i| status.is_none_or(|s| i.status == s)).max_by_key(|i| (number(&i.version).unwrap_or(0), i.timestamp_ms)))
    }

    /// Newest first.
    pub fn list(&self, limit: usize) -> Result<Vec<VersionInfo>> {
        let mut v: Vec<VersionInfo> = self.all()?.into_iter().map(|r| r.info).collect();
        v.sort_by_key(|i| std::cmp::Reverse(number(&i.version).unwrap_or(0)));
        v.truncate(limit);
        Ok(v)
    }

    pub fn text(&self, version: &str) -> Result<Option<String>> {
        Ok(self.load(version)?.map(|r| r.text))
    }

    /// The stored ontology, parsed. The stored hash must still match the text.
    pub fn ontology(&self, version: &str) -> Result<Option<Ontology>> {
        let Some(r) = self.load(version)? else { return Ok(None) };
        if hex(&Sha256::digest(r.text.as_bytes())) != r.info.file_hash {
            return Err(RegistryError::Corrupt(format!("{version}: the stored text no longer matches its hash")));
        }
        Ok(Some(parse(&r.text)?))
    }

    /// Change a version's status. Returns false if there is no such version. Promoting a version retires the one that was
    /// promoted before it, so exactly one version is promoted at a time.
    pub fn set_status(&self, version: &str, status: Status) -> Result<bool> {
        let _g = self.lock.lock().unwrap();
        let Some(mut r) = self.load(version)? else { return Ok(false) };
        if status == Status::Promoted {
            for mut other in self.all()?.into_iter().filter(|o| o.info.status == Status::Promoted && o.info.version != version) {
                other.info.status = Status::Deprecated;
                self.save(&other)?;
            }
        }
        r.info.status = status;
        self.save(&r)?;
        Ok(true)
    }

    pub fn set_consistency(&self, version: &str, result: ConsistencyResult) -> Result<bool> {
        let _g = self.lock.lock().unwrap();
        let Some(mut r) = self.load(version)? else { return Ok(false) };
        r.consistency = Some(result);
        self.save(&r)?;
        Ok(true)
    }

    pub fn consistency(&self, version: &str) -> Result<Option<ConsistencyResult>> {
        Ok(self.load(version)?.and_then(|r| r.consistency))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineer::build;
    use xiar_store::relational::RedbStore;

    fn registry() -> Registry {
        Registry::new(Arc::new(RedbStore::in_memory().unwrap()))
    }

    fn onto(classes: &[&str]) -> Ontology {
        build(&classes.iter().map(|s| s.to_string()).collect(), &Default::default())
    }

    #[test]
    fn versions_are_numbered_stored_hashed_and_listed_newest_first() {
        let r = registry();
        assert!(r.latest(None).unwrap().is_none());
        let v1 = r.register(&onto(&["A"]), "system", false, None, Status::Draft, None).unwrap();
        let v2 = r.register(&onto(&["A", "B"]), "engineer_agent", true, Some("added B".into()), Status::Draft, None).unwrap();
        assert_eq!((v1.version.as_str(), v2.version.as_str()), ("v1", "v2"));
        assert_ne!(v1.file_hash, v2.file_hash);
        assert_eq!(v1.file_hash.len(), 64);
        assert_eq!(r.list(10).unwrap().iter().map(|i| i.version.as_str()).collect::<Vec<_>>(), ["v2", "v1"]);
        assert_eq!(r.list(1).unwrap().len(), 1);
        assert_eq!(r.get("v2").unwrap().unwrap().diff_summary.as_deref(), Some("added B"));
        assert!(r.get("v9").unwrap().is_none());
        assert_eq!(r.ontology("v1").unwrap().unwrap(), onto(&["A"]));
        assert_eq!(r.latest(None).unwrap().unwrap().version, "v2");
    }

    #[test]
    fn latest_can_be_filtered_by_status_and_promotion_retires_the_previous_one() {
        let r = registry();
        for _ in 0..3 {
            r.register(&onto(&["A"]), "s", false, None, Status::Draft, None).unwrap();
        }
        assert!(r.latest(Some(Status::Promoted)).unwrap().is_none());
        assert!(r.set_status("v1", Status::Promoted).unwrap());
        assert_eq!(r.latest(Some(Status::Promoted)).unwrap().unwrap().version, "v1");
        assert!(r.set_status("v3", Status::Promoted).unwrap());
        assert_eq!(r.get("v1").unwrap().unwrap().status, Status::Deprecated, "one promoted version at a time");
        assert_eq!(r.latest(Some(Status::Promoted)).unwrap().unwrap().version, "v3");
        assert!(!r.set_status("v42", Status::Promoted).unwrap());
    }

    #[test]
    fn consistency_results_are_kept_with_the_version() {
        let r = registry();
        r.register(&onto(&["A"]), "s", false, None, Status::Draft, None).unwrap();
        assert!(r.consistency("v1").unwrap().is_none());
        let c = crate::consistency::check(&onto(&["A"]));
        assert!(r.set_consistency("v1", c.clone()).unwrap());
        let back = r.consistency("v1").unwrap().unwrap();
        // JSON floats may differ in the last digit after a round trip, so the timing is compared with a tolerance
        assert!((back.execution_time_ms - c.execution_time_ms).abs() < 1e-6);
        assert_eq!(ConsistencyResult { execution_time_ms: 0.0, ..back }, ConsistencyResult { execution_time_ms: 0.0, ..c.clone() });
        assert!(!r.set_consistency("v2", c).unwrap());
    }

    #[test]
    fn hand_written_text_must_parse_inside_the_profile() {
        let r = registry();
        let ok = r.register_text("Ontology(<http://e> Declaration(Class(<http://e/#A>)))", "manual", false, None, Status::Draft).unwrap();
        assert_eq!(ok.version, "v1");
        let e = r.register_text("Ontology(<http://e> DLSafeRule(Body() Head()))", "manual", false, None, Status::Draft).unwrap_err();
        assert!(e.to_string().contains("DLSafeRule"), "{e}");
        assert_eq!(r.list(10).unwrap().len(), 1, "a rejected text is not stored");
    }

    #[test]
    fn tampering_with_the_stored_text_is_detected() {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let r = Registry::new(store.clone());
        r.register(&onto(&["A"]), "s", false, None, Status::Draft, None).unwrap();
        let raw = store.get(TABLE, b"v1").unwrap().unwrap();
        let mut rec: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        rec["text"] = serde_json::json!(write(&onto(&["Evil"])));
        store.put(TABLE, b"v1", &serde_json::to_vec(&rec).unwrap()).unwrap();
        assert!(matches!(r.ontology("v1"), Err(RegistryError::Corrupt(_))));
    }
}
