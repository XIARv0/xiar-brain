// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Local users and bots. Replaces MIRIX's organization/client/user tables (multi-tenant, API-key based) with the
//! desktop model: several local users, each able to talk to several bots (personas).
//!
//! * Ids are pseudonymous (`user-<uuid>`, `bot-<uuid>`): they are what the audit ledger and filter tags carry.
//! * A user's display name is sealed with that user's key, so after erasure only a tombstone remains.
//! * A bot's `scope` is the `filter_tags.scope` value its memories are written under, which is how per-persona memory
//!   isolation is enforced (a bot is authorised for its own scope only unless the user shares more).
//! * COPPA: memory collection stays off until the age gate is passed (`memory_allowed`). Which path an under-13 user
//!   takes (block or parental consent) is a product decision recorded in the gate state, not decided here.

use crate::ids;
use crate::manager::{MemError, Result, Sealer};
use crate::types::{now_ms, Ms};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use xiar_store::relational::RelationalStore;

const USERS: &str = "reg_users";
const BOTS: &str = "reg_bots";
const ERASED: &str = "reg_erased_users";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgeGate {
    /// not asked yet: no memory collection
    Unknown,
    Adult,
    /// under 13, blocked from memory collection
    UnderThirteenBlocked,
    /// under 13 with recorded parental consent
    UnderThirteenConsented,
}

impl AgeGate {
    pub fn memory_allowed(self) -> bool {
        matches!(self, AgeGate::Adult | AgeGate::UnderThirteenConsented)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub display_name: String,
    /// IANA name such as "Europe/Lisbon" (validated by the settings layer)
    pub timezone: String,
    pub locale: String,
    pub age_gate: AgeGate,
    pub created_ms: Ms,
}

impl User {
    pub fn memory_allowed(&self) -> bool {
        self.age_gate.memory_allowed()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bot {
    pub id: String,
    pub name: String,
    /// prompt name of the persona (`personas/<x>`), resolved through `xiar-prompts`
    pub persona: String,
    /// the user who created it; `None` = shared by every local user
    pub owner: Option<String>,
    /// memory scope this bot writes under (`filter_tags.scope`)
    pub scope: String,
    pub created_ms: Ms,
}

pub struct Registry {
    store: Arc<dyn RelationalStore>,
    sealer: Arc<dyn Sealer>,
    lock: Mutex<()>,
}

fn aad(id: &str) -> Vec<u8> {
    [b"xiar-registry-user:".as_slice(), id.as_bytes()].concat()
}

impl Registry {
    pub fn new(store: Arc<dyn RelationalStore>, sealer: Arc<dyn Sealer>) -> Self {
        Self { store, sealer, lock: Mutex::new(()) }
    }

    fn write_user(&self, u: &User) -> Result<()> {
        let json = serde_json::to_vec(u).map_err(|e| MemError::Serde(e.to_string()))?;
        let sealed = self.sealer.seal(&u.id, &aad(&u.id), &json)?;
        self.store.put(USERS, u.id.as_bytes(), &sealed)?;
        Ok(())
    }

    pub fn create_user(&self, display_name: &str, timezone: &str, locale: &str) -> Result<User> {
        if display_name.trim().is_empty() {
            return Err(MemError::Invalid("display name must not be empty".into()));
        }
        let u = User { id: ids::new_id("user"), display_name: display_name.trim().to_string(), timezone: timezone.to_string(), locale: locale.to_string(), age_gate: AgeGate::Unknown, created_ms: now_ms() };
        let _g = self.lock.lock().unwrap();
        self.write_user(&u)?;
        Ok(u)
    }

    pub fn get_user(&self, id: &str) -> Result<Option<User>> {
        let Some(sealed) = self.store.get(USERS, id.as_bytes())? else { return Ok(None) };
        let json = self.sealer.open(id, &aad(id), &sealed)?;
        serde_json::from_slice(&json).map(Some).map_err(|e| MemError::Serde(e.to_string()))
    }

    pub fn require_user(&self, id: &str) -> Result<User> {
        self.get_user(id)?.ok_or_else(|| MemError::NotFound(format!("user {id}")))
    }

    /// Every readable user (erased users are tombstones and are skipped).
    pub fn list_users(&self) -> Result<Vec<User>> {
        let mut out = Vec::new();
        for (k, _) in self.store.scan(USERS, b"", usize::MAX)? {
            if let Ok(Some(u)) = self.get_user(&String::from_utf8_lossy(&k)) {
                out.push(u);
            }
        }
        out.sort_by_key(|u| u.created_ms);
        Ok(out)
    }

    pub fn update_user<F: FnOnce(&mut User)>(&self, id: &str, f: F) -> Result<User> {
        let _g = self.lock.lock().unwrap();
        let mut u = self.require_user(id)?;
        f(&mut u);
        u.id = id.to_string();
        self.write_user(&u)?;
        Ok(u)
    }

    pub fn set_age_gate(&self, id: &str, gate: AgeGate) -> Result<User> {
        self.update_user(id, |u| u.age_gate = gate)
    }

    pub fn is_erased(&self, id: &str) -> Result<bool> {
        Ok(self.store.get(ERASED, id.as_bytes())?.is_some())
    }

    /// Remove the user record and leave a tombstone (id + time only). Call after `MemoryHub::erase_user`.
    pub fn erase_user(&self, id: &str) -> Result<bool> {
        let _g = self.lock.lock().unwrap();
        let existed = self.store.delete(USERS, id.as_bytes())?;
        self.store.put(ERASED, id.as_bytes(), &now_ms().to_be_bytes())?;
        for b in self.list_bots()? {
            if b.owner.as_deref() == Some(id) {
                self.store.delete(BOTS, b.id.as_bytes())?;
            }
        }
        Ok(existed)
    }

    pub fn create_bot(&self, name: &str, persona: &str, owner: Option<&str>) -> Result<Bot> {
        if name.trim().is_empty() {
            return Err(MemError::Invalid("bot name must not be empty".into()));
        }
        xiar_prompts_name_ok(persona)?;
        if let Some(o) = owner {
            self.require_user(o)?;
        }
        let id = ids::new_id("bot");
        let b = Bot { scope: format!("bot:{id}"), id, name: name.trim().to_string(), persona: persona.to_string(), owner: owner.map(str::to_string), created_ms: now_ms() };
        let json = serde_json::to_vec(&b).map_err(|e| MemError::Serde(e.to_string()))?;
        let _g = self.lock.lock().unwrap();
        self.store.put(BOTS, b.id.as_bytes(), &json)?;
        Ok(b)
    }

    pub fn get_bot(&self, id: &str) -> Result<Option<Bot>> {
        match self.store.get(BOTS, id.as_bytes())? {
            Some(raw) => Ok(Some(serde_json::from_slice(&raw).map_err(|e| MemError::Serde(e.to_string()))?)),
            None => Ok(None),
        }
    }

    pub fn list_bots(&self) -> Result<Vec<Bot>> {
        let mut out: Vec<Bot> = Vec::new();
        for (_, v) in self.store.scan(BOTS, b"", usize::MAX)? {
            out.push(serde_json::from_slice(&v).map_err(|e| MemError::Serde(e.to_string()))?);
        }
        out.sort_by_key(|b| b.created_ms);
        Ok(out)
    }

    /// Bots a user may talk to: their own plus shared ones.
    pub fn bots_for_user(&self, user: &str) -> Result<Vec<Bot>> {
        Ok(self.list_bots()?.into_iter().filter(|b| b.owner.as_deref().is_none_or(|o| o == user)).collect())
    }

    pub fn delete_bot(&self, id: &str) -> Result<bool> {
        Ok(self.store.delete(BOTS, id.as_bytes())?)
    }
}

/// Persona prompt names use the same syntax as `xiar_prompts::validate_name` (lowercase path segments).
fn xiar_prompts_name_ok(name: &str) -> Result<()> {
    let ok = !name.is_empty() && name.len() <= 128 && name.split('/').all(|s| !s.is_empty() && s != ".." && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'));
    if ok { Ok(()) } else { Err(MemError::Invalid(format!("invalid persona prompt name {name:?}"))) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiar_store::keyring::{Keyring, MasterKey};
    use xiar_store::relational::RedbStore;

    fn reg() -> (Arc<RedbStore>, Arc<Keyring>, Registry) {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let ring = Arc::new(Keyring::new(store.clone(), MasterKey::generate()));
        (store.clone(), ring.clone(), Registry::new(store, ring))
    }

    #[test]
    fn users_start_with_memory_off_until_the_age_gate_passes() {
        let (_s, _r, r) = reg();
        let u = r.create_user("Alice", "Europe/Lisbon", "en").unwrap();
        assert!(ids::has_prefix(&u.id, "user"));
        assert!(!u.memory_allowed(), "no memory collection before the gate");
        assert!(r.set_age_gate(&u.id, AgeGate::Adult).unwrap().memory_allowed());
        assert!(!r.set_age_gate(&u.id, AgeGate::UnderThirteenBlocked).unwrap().memory_allowed());
        assert!(r.set_age_gate(&u.id, AgeGate::UnderThirteenConsented).unwrap().memory_allowed());
        assert!(r.create_user("  ", "UTC", "en").is_err());
    }

    #[test]
    fn display_names_are_sealed_and_erasure_leaves_only_a_tombstone() {
        let (s, ring, r) = reg();
        let alice = r.create_user("Alice Wonderland", "UTC", "en").unwrap();
        let bob = r.create_user("Bob", "UTC", "en").unwrap();
        let raw = s.get(USERS, alice.id.as_bytes()).unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Wonderland"), "name is not stored in clear");
        assert_eq!(r.list_users().unwrap().len(), 2);
        assert!(r.erase_user(&alice.id).unwrap());
        ring.shred(&alice.id).unwrap();
        assert!(r.get_user(&alice.id).unwrap().is_none());
        assert!(r.is_erased(&alice.id).unwrap() && !r.is_erased(&bob.id).unwrap());
        assert_eq!(r.list_users().unwrap().iter().map(|u| u.display_name.as_str()).collect::<Vec<_>>(), vec!["Bob"]);
    }

    #[test]
    fn bots_have_scopes_owners_and_visibility() {
        let (_s, _r, r) = reg();
        let alice = r.create_user("Alice", "UTC", "en").unwrap();
        let bob = r.create_user("Bob", "UTC", "en").unwrap();
        let shared = r.create_bot("Helper", "personas/helpful_assistant", None).unwrap();
        let mine = r.create_bot("Alice's analyst", "personas/concise_analyst", Some(&alice.id)).unwrap();
        assert_ne!(shared.scope, mine.scope, "each bot has its own memory scope");
        let ids_for = |u: &str| -> Vec<String> { r.bots_for_user(u).unwrap().into_iter().map(|b| b.name).collect() };
        assert_eq!(ids_for(&alice.id).len(), 2);
        assert_eq!(ids_for(&bob.id), vec!["Helper"], "bob cannot see alice's private bot");
        assert!(r.create_bot("x", "../etc/passwd", None).is_err());
        assert!(r.create_bot("x", "personas/ok", Some("user-missing")).is_err());
        // erasing the owner removes their private bots but not shared ones
        r.erase_user(&alice.id).unwrap();
        assert_eq!(r.list_bots().unwrap().len(), 1);
        assert!(r.delete_bot(&shared.id).unwrap());
    }
}
