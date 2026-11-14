// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Source records: where a memory came from (a screenshot of an app or page, a conversation, an uploaded document, a voice
//! clip). Memory items point at them by id (`Common::source_refs`), so a recalled fact can show where it was learned and an
//! audit can follow it back.
//!
//! Upstream MIRIX has two different tables both called `raw_memory`: on the desktop-agent branch screenshot metadata (app,
//! URL, OCR text), on `main` a context note with a 14-day lifetime. This is the first of them, kept apart from `RawMemory`
//! on purpose: provenance must outlive the 14 days, so it is a store of its own, kept until the user erases it.
//!
//! Privacy: a record holds no pixels and no page text, only the app, a cleaned URL, a label, and a SHA-256 of the captured
//! content so a stored original can be matched later. URLs lose their query string, fragment and credentials (they often
//! carry tokens). Records are sealed with the owner's key, so erasure and crypto-shredding cover them like every memory table.

use crate::manager::{MemError, Result, Sealer};
use crate::types::{now_ms, Ms};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use xiar_store::relational::RelationalStore;

const BY_USER: &str = "src_by_user";
pub const SOURCE_ID_PREFIX: &str = "src";
/// at most this many sources are named in a prompt line (upstream shows the first three)
pub const MAX_SOURCES_SHOWN: usize = 3;
const MAX_APP: usize = 128;
const MAX_URL: usize = 2048;
const MAX_LABEL: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Screenshot,
    Conversation,
    Document,
    Voice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub user_id: String,
    pub kind: SourceKind,
    /// the application it was seen in ("Chrome", "chat", ...)
    pub app: String,
    /// the page address, cleaned (see [`clean_url`])
    pub url: Option<String>,
    /// a session id, file name or window title the user can recognise
    pub label: Option<String>,
    /// SHA-256 (hex) of the captured content, to match a stored original; never the content
    pub content_sha256: Option<String>,
    pub occurred_at_ms: Ms,
    pub created_at_ms: Ms,
}

impl SourceRecord {
    pub fn new(user_id: &str, kind: SourceKind, app: &str) -> Self {
        let now = now_ms();
        Self { id: crate::ids::new_id(SOURCE_ID_PREFIX), user_id: user_id.into(), kind, app: app.trim().into(), url: None, label: None, content_sha256: None, occurred_at_ms: now, created_at_ms: now }
    }

    pub fn with_url(mut self, url: &str) -> Self {
        self.url = clean_url(url);
        self
    }

    pub fn with_label(mut self, label: &str) -> Self {
        let l = label.trim();
        self.label = (!l.is_empty()).then(|| l.chars().take(MAX_LABEL).collect());
        self
    }

    pub fn validate(&self) -> std::result::Result<(), crate::types::ValidationError> {
        if self.app.trim().is_empty() {
            return Err(crate::types::ValidationError::Empty("app"));
        }
        if self.app.chars().count() > MAX_APP {
            return Err(crate::types::ValidationError::TooLong { field: "app", max: MAX_APP });
        }
        if self.url.as_ref().is_some_and(|u| u.chars().count() > MAX_URL) {
            return Err(crate::types::ValidationError::TooLong { field: "url", max: MAX_URL });
        }
        Ok(())
    }

    /// `"Chrome: github.com"`, or just the app when there is no address (upstream's wording).
    pub fn short(&self) -> String {
        match self.url.as_deref().and_then(domain_of) {
            Some(d) => format!("{}: {d}", self.app),
            None => self.app.clone(),
        }
    }
}

/// Keep scheme, host and path; drop credentials, query string and fragment. `None` for anything that is not an http(s) or file address.
pub fn clean_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let (scheme, rest) = raw.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https" | "file") {
        return None;
    }
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or("");
    if host.is_empty() && scheme != "file" {
        return None;
    }
    let out = format!("{scheme}://{host}{path}");
    Some(out.chars().take(MAX_URL).collect())
}

/// The host part of a cleaned address.
pub fn domain_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split('/').next()?;
    (!host.is_empty()).then(|| host.to_string())
}

pub struct SourceManager {
    store: Arc<dyn RelationalStore>,
    sealer: Arc<dyn Sealer>,
    lock: Mutex<()>,
}

fn key(user: &str, id: &str) -> Vec<u8> {
    [user.as_bytes(), b"\0", id.as_bytes()].concat()
}

fn aad(user: &str, id: &str) -> Vec<u8> {
    [b"xiar-source:".as_slice(), user.as_bytes(), b"\0", id.as_bytes()].concat()
}

impl SourceManager {
    pub fn new(store: Arc<dyn RelationalStore>, sealer: Arc<dyn Sealer>) -> Self {
        Self { store, sealer, lock: Mutex::new(()) }
    }

    fn read(&self, user: &str, id: &str) -> Result<Option<SourceRecord>> {
        let Some(sealed) = self.store.get(BY_USER, &key(user, id))? else { return Ok(None) };
        let json = self.sealer.open(user, &aad(user, id), &sealed)?;
        serde_json::from_slice(&json).map(Some).map_err(|e| MemError::Serde(e.to_string()))
    }

    /// Store a record (replacing one with the same id, which is how a restore stays idempotent).
    pub fn create(&self, r: SourceRecord) -> Result<SourceRecord> {
        r.validate()?;
        let _g = self.lock.lock().unwrap();
        let json = serde_json::to_vec(&r).map_err(|e| MemError::Serde(e.to_string()))?;
        let sealed = self.sealer.seal(&r.user_id, &aad(&r.user_id, &r.id), &json)?;
        self.store.put(BY_USER, &key(&r.user_id, &r.id), &sealed)?;
        Ok(r)
    }

    pub fn get(&self, user: &str, id: &str) -> Result<Option<SourceRecord>> {
        self.read(user, id)
    }

    /// Which of `ids` exist for this user (another user's ids never do).
    pub fn existing(&self, user: &str, ids: &[String]) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for id in ids {
            if self.store.get(BY_USER, &key(user, id))?.is_some() {
                out.push(id.clone());
            }
        }
        Ok(out)
    }

    /// Newest first; `limit` 0 means all.
    pub fn list_recent(&self, user: &str, limit: usize) -> Result<Vec<SourceRecord>> {
        let prefix = [user.as_bytes(), b"\0"].concat();
        let mut out = Vec::new();
        for (k, _) in self.store.scan(BY_USER, &prefix, usize::MAX)? {
            if let Some(r) = self.read(user, &String::from_utf8_lossy(&k[prefix.len()..]))? {
                out.push(r);
            }
        }
        out.sort_by(|a, b| b.occurred_at_ms.cmp(&a.occurred_at_ms).then_with(|| a.id.cmp(&b.id)));
        if limit > 0 {
            out.truncate(limit);
        }
        Ok(out)
    }

    pub fn count(&self, user: &str) -> Result<usize> {
        let prefix = [user.as_bytes(), b"\0"].concat();
        Ok(self.store.scan(BY_USER, &prefix, usize::MAX)?.len())
    }

    pub fn delete(&self, user: &str, id: &str) -> Result<bool> {
        let _g = self.lock.lock().unwrap();
        Ok(self.store.delete(BY_USER, &key(user, id))?)
    }

    pub fn delete_user(&self, user: &str) -> Result<usize> {
        let _g = self.lock.lock().unwrap();
        let prefix = [user.as_bytes(), b"\0"].concat();
        let mut n = 0;
        for (k, _) in self.store.scan(BY_USER, &prefix, usize::MAX)? {
            if self.store.delete(BY_USER, &k)? {
                n += 1;
            }
        }
        Ok(n)
    }

    /// ` [Sources: Chrome: github.com, Safari: docs.python.org]` for the first [`MAX_SOURCES_SHOWN`] references that still exist;
    /// empty when there are none. A reference to a record that was deleted is skipped (the item keeps the id for the audit).
    pub fn describe(&self, user: &str, refs: &[String]) -> String {
        let shown: Vec<String> = refs.iter().take(MAX_SOURCES_SHOWN).filter_map(|id| self.read(user, id).ok().flatten()).map(|r| r.short()).collect();
        if shown.is_empty() {
            String::new()
        } else {
            format!(" [Sources: {}]", shown.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiar_store::keyring::{Keyring, MasterKey};
    use xiar_store::relational::RedbStore;

    fn mgr() -> (Arc<Keyring>, SourceManager) {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let ring = Arc::new(Keyring::new(store.clone(), MasterKey::generate()));
        (ring.clone(), SourceManager::new(store, ring))
    }

    #[test]
    fn urls_lose_credentials_query_and_fragment() {
        assert_eq!(clean_url("https://user:pw@GitHub.com/a/b?token=secret#top").as_deref(), Some("https://GitHub.com/a/b"));
        assert_eq!(clean_url("http://example.org").as_deref(), Some("http://example.org"));
        assert_eq!(clean_url("javascript://x"), None);
        assert_eq!(clean_url("not a url"), None);
        assert_eq!(clean_url("https:///nohost"), None);
    }

    #[test]
    fn the_prompt_line_names_app_and_domain_and_stops_at_three() {
        let (_r, m) = mgr();
        let mut ids = vec![];
        for (app, url) in [("Chrome", Some("https://github.com/x?y=1")), ("Notes", None), ("Safari", Some("https://docs.python.org/3/")), ("Edge", Some("https://fourth.example/"))] {
            let mut r = SourceRecord::new("u", SourceKind::Screenshot, app);
            if let Some(u) = url {
                r = r.with_url(u);
            }
            ids.push(m.create(r).unwrap().id);
        }
        assert_eq!(m.describe("u", &ids), " [Sources: Chrome: github.com, Notes, Safari: docs.python.org]");
        assert_eq!(m.describe("u", &[]), "");
        assert_eq!(m.describe("someone-else", &ids), "", "another user's ids describe nothing");
        m.delete("u", &ids[0]).unwrap();
        assert_eq!(m.describe("u", &ids), " [Sources: Notes, Safari: docs.python.org]", "a deleted record is skipped");
    }

    #[test]
    fn records_are_user_scoped_sealed_and_erasable() {
        let (_ring, m) = mgr();
        let a = m.create(SourceRecord::new("alice", SourceKind::Conversation, "chat").with_label("session-1")).unwrap();
        m.create(SourceRecord::new("bob", SourceKind::Document, "Files").with_label("notes.pdf")).unwrap();
        assert_eq!(m.get("alice", &a.id).unwrap().unwrap().label.as_deref(), Some("session-1"));
        assert!(m.get("bob", &a.id).unwrap().is_none(), "bob cannot read alice's record");
        assert_eq!(m.existing("bob", std::slice::from_ref(&a.id)).unwrap(), Vec::<String>::new());
        assert_eq!((m.count("alice").unwrap(), m.count("bob").unwrap()), (1, 1));
        assert_eq!(m.delete_user("alice").unwrap(), 1);
        assert_eq!((m.count("alice").unwrap(), m.count("bob").unwrap()), (0, 1));
    }

    #[test]
    fn an_empty_app_is_refused() {
        let (_r, m) = mgr();
        assert!(m.create(SourceRecord::new("u", SourceKind::Voice, "  ")).is_err());
    }
}
