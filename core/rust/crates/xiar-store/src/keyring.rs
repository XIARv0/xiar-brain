// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! At-rest encryption with per-user data keys (DEKs).
//!
//! * Each user gets a random 256-bit DEK, stored in the relational store **wrapped** (encrypted) by a master key
//!   that the caller obtains from the OS keychain (never stored in files or config).
//! * `seal`/`open` use XChaCha20-Poly1305 (192-bit random nonces, safe for random generation) with caller
//!   supplied associated data, so a ciphertext cannot be moved to another record/user undetected.
//! * `shred(user)` deletes the wrapped DEK. Everything sealed for that user becomes permanently unreadable
//!   (`StoreError::KeyErased`) while ciphertext stays in place, so hash chains over it keep verifying.

use crate::relational::RelationalStore;
use crate::{Result, StoreError};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    Key, XChaCha20Poly1305, XNonce,
};
use rand::RngCore;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

const KEYS_TABLE: &str = "sys_user_keys";
const LOCKS_TABLE: &str = "sys_user_locks";
const RECOVERY_TABLE: &str = "sys_user_recovery";
const NONCE_LEN: usize = 24;
const SALT_LEN: usize = 16;
const CUSTODY_TABLE: &str = "sys_user_custody";

#[derive(Clone)]
pub struct MasterKey(Zeroizing<[u8; 32]>);

impl MasterKey {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(Zeroizing::new(b))
    }
    pub fn generate() -> Self {
        let mut b = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut b);
        Self::from_bytes(b)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

pub struct Keyring {
    store: Arc<dyn RelationalStore>,
    master: MasterKey,
    cache: Mutex<HashMap<String, Zeroizing<[u8; 32]>>>,
    /// Argon2id cost (memory in KiB, passes) of the keys made from a password
    kdf: (u32, u32),
}

fn aead_seal(key: &[u8; 32], aad: &[u8], pt: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ct = XChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: pt, aad })
        .expect("encryption of in-memory buffer cannot fail");
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    out
}

fn aead_open(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < NONCE_LEN + 16 {
        return Err(StoreError::Malformed("sealed value too short"));
    }
    let (nonce, ct) = sealed.split_at(NONCE_LEN);
    XChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
        .map_err(|_| StoreError::Decrypt)
}

impl Keyring {
    pub fn new(store: Arc<dyn RelationalStore>, master: MasterKey) -> Self {
        Self { store, master, cache: Mutex::new(HashMap::new()), kdf: (64 * 1024, 3) }
    }

    /// Lighter password hashing, for tests.
    pub fn with_kdf(mut self, memory_kib: u32, passes: u32) -> Self {
        self.kdf = (memory_kib, passes);
        self
    }

    fn derive(&self, secret: &str, salt: &[u8], (m, t): (u32, u32)) -> Result<Zeroizing<[u8; 32]>> {
        let params = argon2::Params::new(m, t, 1, Some(32)).map_err(|_| StoreError::Malformed("key derivation settings"))?;
        let mut out = Zeroizing::new([0u8; 32]);
        argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
            .hash_password_into(secret.as_bytes(), salt, &mut *out)
            .map_err(|_| StoreError::Malformed("key derivation"))?;
        Ok(out)
    }

    fn lock_aad(user: &str, table: &str) -> Vec<u8> {
        [b"xiar-dek-lock:".as_slice(), table.as_bytes(), b":", user.as_bytes()].concat()
    }

    /// `[salt 16][memory KiB u32][passes u32][sealed key]`
    fn make_wrap(&self, table: &str, user: &str, secret: &str, dek: &[u8; 32]) -> Result<Vec<u8>> {
        let mut salt = [0u8; SALT_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        let key = self.derive(secret, &salt, self.kdf)?;
        let mut out = salt.to_vec();
        out.extend_from_slice(&self.kdf.0.to_le_bytes());
        out.extend_from_slice(&self.kdf.1.to_le_bytes());
        out.extend_from_slice(&aead_seal(&key, &Self::lock_aad(user, table), dek));
        Ok(out)
    }

    fn open_wrap(&self, table: &str, user: &str, secret: &str) -> Result<Zeroizing<[u8; 32]>> {
        let w = self.store.get(table, user.as_bytes())?.ok_or_else(|| StoreError::Locked(user.to_string()))?;
        if w.len() < SALT_LEN + 8 + NONCE_LEN + 16 {
            return Err(StoreError::Malformed("lock wrap too short"));
        }
        let (salt, rest) = w.split_at(SALT_LEN);
        let m = u32::from_le_bytes(rest[0..4].try_into().unwrap());
        let t = u32::from_le_bytes(rest[4..8].try_into().unwrap());
        let key = self.derive(secret, salt, (m, t))?;
        let raw = aead_open(&key, &Self::lock_aad(user, table), &rest[8..])?;
        let dek: [u8; 32] = raw.try_into().map_err(|_| StoreError::Malformed("DEK length"))?;
        Ok(Zeroizing::new(dek))
    }

    /// Whether this user's data is locked with a password.
    pub fn is_locked_with_password(&self, user: &str) -> Result<bool> {
        Ok(self.store.get(LOCKS_TABLE, user.as_bytes())?.is_some())
    }

    /// Locks the user's data with a password (and a recovery key where there is one). The key must be open now (it is made if the user has
    /// none yet). The password wrap is proven to open the key before the copy that the master key wraps is deleted.
    pub fn lock_with_password(&self, user: &str, password: &str, recovery: Option<&str>) -> Result<()> {
        let dek = self.dek_or_create(user)?;
        let wrap = self.make_wrap(LOCKS_TABLE, user, password, &dek)?;
        self.store.put(LOCKS_TABLE, user.as_bytes(), &wrap)?;
        if self.open_wrap(LOCKS_TABLE, user, password)?.as_slice() != dek.as_slice() {
            self.store.delete(LOCKS_TABLE, user.as_bytes())?;
            return Err(StoreError::Malformed("the password wrap did not open the key"));
        }
        if let Some(r) = recovery {
            self.cache.lock().unwrap().insert(user.to_string(), dek.clone());
            self.set_recovery(user, r)?;
        }
        self.store.delete(KEYS_TABLE, user.as_bytes())?;
        self.cache.lock().unwrap().insert(user.to_string(), dek);
        Ok(())
    }

    /// Opens a locked user's key with the password, and keeps it in memory until `forget`.
    pub fn unlock(&self, user: &str, password: &str) -> Result<()> {
        let dek = self.open_wrap(LOCKS_TABLE, user, password)?;
        self.cache.lock().unwrap().insert(user.to_string(), dek);
        Ok(())
    }

    /// Opens a locked user's key with the recovery key.
    pub fn unlock_with_recovery(&self, user: &str, recovery: &str) -> Result<()> {
        let dek = self.open_wrap(RECOVERY_TABLE, user, recovery)?;
        self.cache.lock().unwrap().insert(user.to_string(), dek);
        Ok(())
    }

    /// Replaces the password the key is wrapped by. The key must be open (the account is signed in).
    pub fn change_password(&self, user: &str, new_password: &str) -> Result<()> {
        let dek = self.load_dek(user)?.ok_or_else(|| StoreError::KeyErased(user.to_string()))?;
        let wrap = self.make_wrap(LOCKS_TABLE, user, new_password, &dek)?;
        self.store.put(LOCKS_TABLE, user.as_bytes(), &wrap)
    }

    /// Replaces the recovery key the key is wrapped by. The key must be open.
    pub fn set_recovery(&self, user: &str, recovery: &str) -> Result<()> {
        let dek = self.load_dek(user)?.ok_or_else(|| StoreError::KeyErased(user.to_string()))?;
        let wrap = self.make_wrap(RECOVERY_TABLE, user, recovery, &dek)?;
        self.store.put(RECOVERY_TABLE, user.as_bytes(), &wrap)
    }

    /// Whether a recovery key can open this user's data.
    pub fn has_recovery(&self, user: &str) -> Result<bool> {
        Ok(self.store.get(RECOVERY_TABLE, user.as_bytes())?.is_some())
    }

    /// Whether the user's key is open now (signed in, or not locked with a password at all).
    pub fn is_open(&self, user: &str) -> bool {
        self.load_dek(user).is_ok_and(|k| k.is_some())
    }

    fn custody_aad(user: &str, custodian: &str) -> Vec<u8> {
        format!("xiar-dek-custody:{user}:{custodian}").into_bytes()
    }

    /// A parent's custody of a child's data: the child's key is also wrapped by the parent's key, so the parent (signed in) can open it
    /// and set a new password if the child forgets theirs. Both keys must be open.
    pub fn set_custody(&self, user: &str, custodian: &str) -> Result<()> {
        let dek = self.load_dek(user)?.ok_or_else(|| StoreError::KeyErased(user.to_string()))?;
        let cust = self.load_dek(custodian)?.ok_or_else(|| StoreError::KeyErased(custodian.to_string()))?;
        self.store.put(CUSTODY_TABLE, user.as_bytes(), &aead_seal(&cust, &Self::custody_aad(user, custodian), &*dek))
    }

    pub fn drop_recovery(&self, user: &str) -> Result<bool> {
        self.store.delete(RECOVERY_TABLE, user.as_bytes())
    }

    pub fn has_custody(&self, user: &str) -> Result<bool> {
        Ok(self.store.get(CUSTODY_TABLE, user.as_bytes())?.is_some())
    }

    pub fn drop_custody(&self, user: &str) -> Result<bool> {
        self.store.delete(CUSTODY_TABLE, user.as_bytes())
    }

    /// Opens a child's key with the parent's (which must be open).
    pub fn unlock_by_custodian(&self, user: &str, custodian: &str) -> Result<()> {
        let cust = self.load_dek(custodian)?.ok_or_else(|| StoreError::KeyErased(custodian.to_string()))?;
        let w = self.store.get(CUSTODY_TABLE, user.as_bytes())?.ok_or_else(|| StoreError::Locked(user.to_string()))?;
        let raw = aead_open(&cust, &Self::custody_aad(user, custodian), &w)?;
        let dek: [u8; 32] = raw.try_into().map_err(|_| StoreError::Malformed("DEK length"))?;
        self.cache.lock().unwrap().insert(user.to_string(), Zeroizing::new(dek));
        Ok(())
    }

    /// Takes the user's key out of memory: a locked user's data can not be opened until `unlock`. A key that is not locked with a
    /// password stays reachable (the master key opens it again).
    pub fn forget(&self, user: &str) {
        self.cache.lock().unwrap().remove(user);
    }

    fn wrap_aad(user: &str) -> Vec<u8> {
        [b"xiar-dek-wrap:".as_slice(), user.as_bytes()].concat()
    }

    fn load_dek(&self, user: &str) -> Result<Option<Zeroizing<[u8; 32]>>> {
        if let Some(k) = self.cache.lock().unwrap().get(user) {
            return Ok(Some(k.clone()));
        }
        let Some(wrapped) = self.store.get(KEYS_TABLE, user.as_bytes())? else {
            // no copy wrapped by the master key: either no key at all, or the password holds the only way in
            return if self.is_locked_with_password(user)? { Err(StoreError::Locked(user.to_string())) } else { Ok(None) };
        };
        let raw = aead_open(self.master.as_bytes(), &Self::wrap_aad(user), &wrapped)?;
        let dek: [u8; 32] = raw.try_into().map_err(|_| StoreError::Malformed("DEK length"))?;
        let dek = Zeroizing::new(dek);
        self.cache.lock().unwrap().insert(user.to_string(), dek.clone());
        Ok(Some(dek))
    }

    /// Existing DEK, or create one. Creation is atomic so concurrent first-use cannot produce two keys.
    fn dek_or_create(&self, user: &str) -> Result<Zeroizing<[u8; 32]>> {
        if let Some(k) = self.load_dek(user)? {
            return Ok(k);
        }
        let mut fresh = Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng.fill_bytes(&mut *fresh);
        let wrapped = aead_seal(self.master.as_bytes(), &Self::wrap_aad(user), &*fresh);
        let mut created = false;
        self.store.write(&mut |t| {
            created = t.get(KEYS_TABLE, user.as_bytes())?.is_none();
            if created {
                t.put(KEYS_TABLE, user.as_bytes(), &wrapped)?;
            }
            Ok(())
        })?;
        if created {
            self.cache.lock().unwrap().insert(user.to_string(), fresh.clone());
            Ok(fresh)
        } else {
            self.load_dek(user)?.ok_or(StoreError::Malformed("key vanished"))
        }
    }

    /// Encrypt `plaintext` for `user`. `aad` must be supplied again to open (bind it to the record id/kind).
    pub fn seal(&self, user: &str, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        let dek = self.dek_or_create(user)?;
        Ok(aead_seal(&dek, aad, plaintext))
    }

    pub fn open(&self, user: &str, aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
        let dek = self.load_dek(user)?.ok_or_else(|| StoreError::KeyErased(user.to_string()))?;
        aead_open(&dek, aad, sealed)
    }

    /// Crypto-shred: permanently delete the user's key. Irreversible.
    pub fn shred(&self, user: &str) -> Result<bool> {
        self.cache.lock().unwrap().remove(user);
        let a = self.store.delete(LOCKS_TABLE, user.as_bytes())?;
        let b = self.store.delete(RECOVERY_TABLE, user.as_bytes())?;
        let c = self.store.delete(CUSTODY_TABLE, user.as_bytes())?;
        Ok(self.store.delete(KEYS_TABLE, user.as_bytes())? || a || b || c)
    }

    pub fn has_key(&self, user: &str) -> Result<bool> {
        match self.load_dek(user) {
            Ok(k) => Ok(k.is_some()),
            Err(StoreError::Locked(_)) => Ok(true),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relational::RedbStore;

    fn ring() -> (Arc<RedbStore>, Keyring) {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let k = Keyring::new(store.clone(), MasterKey::generate());
        (store, k)
    }

    #[test]
    fn seal_open_roundtrip_and_aad_binding() {
        let (_s, k) = ring();
        let ct = k.seal("alice", b"rec:1", b"secret").unwrap();
        assert_eq!(k.open("alice", b"rec:1", &ct).unwrap(), b"secret");
        assert!(matches!(k.open("alice", b"rec:2", &ct), Err(StoreError::Decrypt)), "wrong AAD rejected");
        let mut bad = ct.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(matches!(k.open("alice", b"rec:1", &bad), Err(StoreError::Decrypt)), "tamper rejected");
    }

    #[test]
    fn users_have_independent_keys() {
        let (_s, k) = ring();
        let ct = k.seal("alice", b"x", b"a").unwrap();
        k.seal("bob", b"x", b"b").unwrap();
        assert!(matches!(k.open("bob", b"x", &ct), Err(StoreError::Decrypt)));
    }

    #[test]
    fn shred_makes_data_unrecoverable_but_others_unaffected() {
        let (_s, k) = ring();
        let a = k.seal("alice", b"x", b"a").unwrap();
        let b = k.seal("bob", b"x", b"b").unwrap();
        assert!(k.shred("alice").unwrap());
        assert!(matches!(k.open("alice", b"x", &a), Err(StoreError::KeyErased(_))));
        assert_eq!(k.open("bob", b"x", &b).unwrap(), b"b");
        assert!(!k.has_key("alice").unwrap());
        // a later write for the same user id creates a NEW key; the old ciphertext stays unreadable
        let a2 = k.seal("alice", b"x", b"new").unwrap();
        assert_eq!(k.open("alice", b"x", &a2).unwrap(), b"new");
        assert!(matches!(k.open("alice", b"x", &a), Err(StoreError::Decrypt)));
    }

    #[test]
    fn dek_survives_restart_only_with_the_same_master_key() {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let master = MasterKey::generate();
        let ct = Keyring::new(store.clone(), master.clone()).seal("alice", b"x", b"hello").unwrap();
        assert_eq!(Keyring::new(store.clone(), master).open("alice", b"x", &ct).unwrap(), b"hello");
        let wrong = Keyring::new(store, MasterKey::generate());
        assert!(matches!(wrong.open("alice", b"x", &ct), Err(StoreError::Decrypt)), "wrong master key cannot unwrap DEK");
    }

    fn light_ring() -> (Arc<RedbStore>, Keyring) {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let k = Keyring::new(store.clone(), MasterKey::generate()).with_kdf(8, 1);
        (store, k)
    }

    #[test]
    fn a_locked_account_opens_only_with_its_password() {
        let (s, k) = light_ring();
        // data sealed before the lock (the migration case) stays readable after it
        let ct = k.seal("anna", b"x", b"old data").unwrap();
        k.lock_with_password("anna", "correct horse", Some("AAAAA-BBBBB-CCCCC-DDDDD")).unwrap();
        assert!(k.is_locked_with_password("anna").unwrap());
        assert!(s.get(KEYS_TABLE, b"anna").unwrap().is_none(), "the master key no longer opens it");
        assert_eq!(k.open("anna", b"x", &ct).unwrap(), b"old data", "signed in right after locking");
        // signing out
        k.forget("anna");
        assert!(matches!(k.open("anna", b"x", &ct), Err(StoreError::Locked(_))));
        assert!(matches!(k.seal("anna", b"x", b"new"), Err(StoreError::Locked(_))), "no fresh key is made behind the lock");
        assert!(k.has_key("anna").unwrap());
        assert!(matches!(k.unlock("anna", "wrong"), Err(StoreError::Decrypt)));
        k.unlock("anna", "correct horse").unwrap();
        assert_eq!(k.open("anna", b"x", &ct).unwrap(), b"old data");
    }

    #[test]
    fn a_password_change_and_the_recovery_key_keep_the_data() {
        let (_s, k) = light_ring();
        let ct = k.seal("anna", b"x", b"data").unwrap();
        k.lock_with_password("anna", "first password", Some("RECOV-ERYKE-YYYYY-ZZZZZ")).unwrap();
        k.change_password("anna", "second password").unwrap();
        k.forget("anna");
        assert!(matches!(k.unlock("anna", "first password"), Err(StoreError::Decrypt)));
        k.unlock("anna", "second password").unwrap();
        // a forgotten password: the recovery key opens it, then a new password is set
        k.forget("anna");
        assert!(matches!(k.unlock_with_recovery("anna", "wrong"), Err(StoreError::Decrypt)));
        k.unlock_with_recovery("anna", "RECOV-ERYKE-YYYYY-ZZZZZ").unwrap();
        k.change_password("anna", "third password").unwrap();
        k.forget("anna");
        k.unlock("anna", "third password").unwrap();
        assert_eq!(k.open("anna", b"x", &ct).unwrap(), b"data");
    }

    #[test]
    fn a_parent_can_open_a_childs_data_and_a_grown_up_child_is_free_of_it() {
        let (_s, k) = light_ring();
        k.lock_with_password("anna", "annas password", Some("RECOV-ERYKE-YYYYY-ZZZZZ")).unwrap();
        let ct = k.seal("kim", b"x", b"kim data").unwrap();
        k.lock_with_password("kim", "kims long password", None).unwrap();
        k.set_custody("kim", "anna").unwrap();
        k.forget("kim");
        assert!(matches!(k.open("kim", b"x", &ct), Err(StoreError::Locked(_))));
        k.unlock_by_custodian("kim", "anna").unwrap();
        k.change_password("kim", "a new password").unwrap();
        k.forget("kim");
        k.unlock("kim", "a new password").unwrap();
        assert_eq!(k.open("kim", b"x", &ct).unwrap(), b"kim data");
        // custody ends: the owner can not open it any more
        k.drop_custody("kim").unwrap();
        k.forget("kim");
        assert!(k.unlock_by_custodian("kim", "anna").is_err());
        assert!(k.has_recovery("anna").unwrap() && !k.has_recovery("kim").unwrap());
    }

    #[test]
    fn shredding_a_locked_account_removes_every_way_in() {
        let (s, k) = light_ring();
        let ct = k.seal("kim", b"x", b"data").unwrap();
        k.lock_with_password("kim", "kims long password", Some("RECOV-ERYKE-YYYYY-ZZZZZ")).unwrap();
        assert!(k.shred("kim").unwrap());
        for t in [KEYS_TABLE, LOCKS_TABLE, RECOVERY_TABLE] {
            assert!(s.get(t, b"kim").unwrap().is_none(), "{t}");
        }
        assert!(matches!(k.open("kim", b"x", &ct), Err(StoreError::KeyErased(_))));
    }

    #[test]
    fn wrapped_dek_is_not_stored_in_clear() {
        let (s, k) = ring();
        k.seal("alice", b"x", b"a").unwrap();
        let stored = s.get(KEYS_TABLE, b"alice").unwrap().unwrap();
        assert_eq!(stored.len(), NONCE_LEN + 32 + 16);
    }
}
