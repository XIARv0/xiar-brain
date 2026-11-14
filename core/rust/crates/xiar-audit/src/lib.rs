// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! xiar-audit — append-only, hash-chained, tamper-evident session/event ledger.
//!
//! * Every entry (message, LLM call, tool call, memory read/write, Laya decision, UI-variable change...) gets a
//!   sequence number, timestamp, session id, pseudonymous user id, bot id, kind and tags, plus an encrypted
//!   payload, and is chained to the previous entry with SHA-256.
//! * Metadata is stored in clear (needed for audit queries and the chain). **Use pseudonymous ids for users and
//!   bots, never names or emails.** Personal content lives only in the payload, sealed with the user's key.
//! * The hash covers the *ciphertext*, so crypto-shredding a user (deleting their key) leaves the chain valid
//!   while the content becomes unrecoverable (erasure vs. audit).
//! * Optional ed25519-signed checkpoints anchor the chain head, so truncating the tail or rewriting history with
//!   a recomputed chain is detected.
//! * Nothing here ever deletes or edits entries; retention/pruning of other stores never touches the ledger.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use xiar_store::keyring::Keyring;
use xiar_store::relational::{RelationalStore, Txn};
use xiar_store::StoreError;

const LEDGER: &str = "audit_ledger";
const BY_SESSION: &str = "audit_by_session";
const SESSIONS: &str = "audit_sessions";
const CHECKPOINTS: &str = "audit_checkpoints";
const GENESIS: [u8; 32] = [0u8; 32];
const DOMAIN: &[u8] = b"XIAR-AUDIT-v1";

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("ledger integrity violation at seq {seq}: {reason}")]
    Integrity { seq: u64, reason: String },
    #[error("entry {0} not found")]
    NotFound(u64),
    #[error("malformed ledger record")]
    Malformed,
}

pub type Result<T> = std::result::Result<T, AuditError>;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Public metadata of a stored entry (everything except the decrypted payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMeta {
    pub seq: u64,
    pub ts_ms: u64,
    pub session: String,
    pub user: String,
    pub bot: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub hash: [u8; 32],
}

pub struct NewEntry<'a> {
    pub session: &'a str,
    pub user: &'a str,
    pub bot: &'a str,
    pub kind: &'a str,
    pub tags: &'a [&'a str],
    /// Content to store (sealed with the user's key). Empty is allowed.
    pub payload: &'a [u8],
    pub ts_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub entries: u64,
    pub head_hash: Option<[u8; 32]>,
    pub checkpoints_verified: u64,
}

struct Stored {
    meta: EntryMeta,
    sealed: Vec<u8>,
    prev: [u8; 32],
}

// ---- tiny length-prefixed codec (avoids JSON bloat; stable, explicit) ----
struct W(Vec<u8>);
impl W {
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(&(b.len() as u32).to_be_bytes());
        self.0.extend_from_slice(b);
    }
}
struct R<'a>(&'a [u8]);
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.0.len() < n {
            return Err(AuditError::Malformed);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
        self.take(n)
    }
    fn string(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| AuditError::Malformed)
    }
}

fn entry_hash(prev: &[u8; 32], m: &EntryMeta, sealed: &[u8]) -> [u8; 32] {
    let mut w = W(Vec::new());
    w.bytes(DOMAIN);
    w.bytes(prev);
    w.u64(m.seq);
    w.u64(m.ts_ms);
    for s in [&m.session, &m.user, &m.bot, &m.kind] {
        w.bytes(s.as_bytes());
    }
    w.u64(m.tags.len() as u64);
    for t in &m.tags {
        w.bytes(t.as_bytes());
    }
    w.bytes(sealed); // ciphertext, not plaintext: survives crypto-shredding
    Sha256::digest(&w.0).into()
}

fn encode(s: &Stored) -> Vec<u8> {
    let m = &s.meta;
    let mut w = W(Vec::new());
    w.u64(m.seq);
    w.u64(m.ts_ms);
    for x in [&m.session, &m.user, &m.bot, &m.kind] {
        w.bytes(x.as_bytes());
    }
    w.u64(m.tags.len() as u64);
    for t in &m.tags {
        w.bytes(t.as_bytes());
    }
    w.bytes(&s.sealed);
    w.bytes(&s.prev);
    w.bytes(&m.hash);
    w.0
}

fn decode(raw: &[u8]) -> Result<Stored> {
    let mut r = R(raw);
    let seq = r.u64()?;
    let ts_ms = r.u64()?;
    let (session, user, bot, kind) = (r.string()?, r.string()?, r.string()?, r.string()?);
    let n = r.u64()? as usize;
    if n > 4096 {
        return Err(AuditError::Malformed);
    }
    let mut tags = Vec::with_capacity(n);
    for _ in 0..n {
        tags.push(r.string()?);
    }
    let sealed = r.bytes()?.to_vec();
    let prev: [u8; 32] = r.bytes()?.try_into().map_err(|_| AuditError::Malformed)?;
    let hash: [u8; 32] = r.bytes()?.try_into().map_err(|_| AuditError::Malformed)?;
    Ok(Stored { meta: EntryMeta { seq, ts_ms, session, user, bot, kind, tags, hash }, sealed, prev })
}

fn aad(user: &str, session: &str, kind: &str) -> Vec<u8> {
    let mut w = W(b"xiar-audit-payload".to_vec());
    for s in [user, session, kind] {
        w.bytes(s.as_bytes());
    }
    w.0
}

fn seq_key(seq: u64) -> [u8; 8] {
    seq.to_be_bytes()
}

pub struct Ledger {
    store: Arc<dyn RelationalStore>,
    keyring: Arc<Keyring>,
    signer: Option<SigningKey>,
}

impl Ledger {
    pub fn new(store: Arc<dyn RelationalStore>, keyring: Arc<Keyring>) -> Self {
        Self { store, keyring, signer: None }
    }

    /// Enable signed checkpoints. The key must be kept outside the data store (OS keychain).
    pub fn with_signer(mut self, key: SigningKey) -> Self {
        self.signer = Some(key);
        self
    }

    pub fn generate_signing_key() -> SigningKey {
        SigningKey::generate(&mut rand::rngs::OsRng)
    }

    /// Start a session and return its random id.
    pub fn start_session(&self, user: &str, bot: &str, tags: &[&str]) -> Result<String> {
        let mut b = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut b);
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let mut w = W(Vec::new());
        w.bytes(user.as_bytes());
        w.bytes(bot.as_bytes());
        w.u64(now_ms());
        w.u64(0); // ended_ms (0 = open)
        w.u64(tags.len() as u64);
        for t in tags {
            w.bytes(t.as_bytes());
        }
        self.store.put(SESSIONS, id.as_bytes(), &w.0)?;
        self.append(&NewEntry { session: &id, user, bot, kind: "session_start", tags, payload: b"", ts_ms: now_ms() })?;
        Ok(id)
    }

    pub fn end_session(&self, session: &str) -> Result<()> {
        let raw = self.store.get(SESSIONS, session.as_bytes())?.ok_or(AuditError::Malformed)?;
        let mut r = R(&raw);
        let (user, bot) = (r.string()?, r.string()?);
        let started = r.u64()?;
        let _ended = r.u64()?;
        let n = r.u64()? as usize;
        let mut tags = Vec::new();
        for _ in 0..n {
            tags.push(r.string()?);
        }
        let end = now_ms();
        let mut w = W(Vec::new());
        w.bytes(user.as_bytes());
        w.bytes(bot.as_bytes());
        w.u64(started);
        w.u64(end);
        w.u64(tags.len() as u64);
        for t in &tags {
            w.bytes(t.as_bytes());
        }
        self.store.put(SESSIONS, session.as_bytes(), &w.0)?;
        let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();
        self.append(&NewEntry { session, user: &user, bot: &bot, kind: "session_end", tags: &tag_refs, payload: b"", ts_ms: end })?;
        Ok(())
    }

    /// Append an entry atomically (sequence number and previous hash are read inside the write transaction).
    pub fn append(&self, e: &NewEntry<'_>) -> Result<EntryMeta> {
        let sealed = self.keyring.seal(e.user, &aad(e.user, e.session, e.kind), e.payload)?;
        let mut out = None;
        self.store.write(&mut |t: &mut dyn Txn| {
            let (seq, prev) = match t.last(LEDGER)? {
                Some((k, v)) => {
                    let last = decode(&v).map_err(|_| StoreError::Malformed("ledger tail"))?;
                    debug_assert_eq!(k, seq_key(last.meta.seq));
                    (last.meta.seq + 1, last.meta.hash)
                }
                None => (0, GENESIS),
            };
            let mut meta = EntryMeta {
                seq,
                ts_ms: e.ts_ms,
                session: e.session.to_string(),
                user: e.user.to_string(),
                bot: e.bot.to_string(),
                kind: e.kind.to_string(),
                tags: e.tags.iter().map(|s| s.to_string()).collect(),
                hash: [0; 32],
            };
            meta.hash = entry_hash(&prev, &meta, &sealed);
            let stored = Stored { meta: meta.clone(), sealed: sealed.clone(), prev };
            t.put(LEDGER, &seq_key(seq), &encode(&stored))?;
            let mut idx = e.session.as_bytes().to_vec();
            idx.push(0);
            idx.extend_from_slice(&seq_key(seq));
            t.put(BY_SESSION, &idx, b"")?;
            out = Some(meta);
            Ok(())
        })?;
        Ok(out.expect("write closure ran"))
    }

    fn load(&self, seq: u64) -> Result<Stored> {
        let raw = self.store.get(LEDGER, &seq_key(seq))?.ok_or(AuditError::NotFound(seq))?;
        decode(&raw)
    }

    pub fn meta(&self, seq: u64) -> Result<EntryMeta> {
        Ok(self.load(seq)?.meta)
    }

    /// Decrypt an entry's payload. Fails with `KeyErased` after the user was crypto-shredded.
    pub fn read_payload(&self, seq: u64) -> Result<Vec<u8>> {
        let s = self.load(seq)?;
        Ok(self.keyring.open(&s.meta.user, &aad(&s.meta.user, &s.meta.session, &s.meta.kind), &s.sealed)?)
    }

    pub fn head(&self) -> Result<Option<EntryMeta>> {
        let mut out = None;
        self.store.read(&mut |t| {
            if let Some((_, v)) = t.last(LEDGER)? {
                out = Some(decode(&v).map_err(|_| StoreError::Malformed("ledger tail"))?.meta);
            }
            Ok(())
        })?;
        Ok(out)
    }

    pub fn session_entries(&self, session: &str) -> Result<Vec<EntryMeta>> {
        let mut prefix = session.as_bytes().to_vec();
        prefix.push(0);
        let rows = self.store.scan(BY_SESSION, &prefix, usize::MAX)?;
        rows.iter().map(|(k, _)| self.meta(u64::from_be_bytes(k[prefix.len()..].try_into().map_err(|_| AuditError::Malformed)?))).collect()
    }

    /// Sign the current head. No-op (Ok(None)) when no signer is configured or the ledger is empty.
    pub fn checkpoint(&self) -> Result<Option<u64>> {
        let (Some(sk), Some(head)) = (&self.signer, self.head()?) else { return Ok(None) };
        let sig = sk.sign(&checkpoint_msg(head.seq, &head.hash));
        let mut v = head.hash.to_vec();
        v.extend_from_slice(&sig.to_bytes());
        self.store.put(CHECKPOINTS, &seq_key(head.seq), &v)?;
        Ok(Some(head.seq))
    }

    /// Verify the whole chain: contiguous sequence numbers, every hash recomputed from stored fields and the
    /// previous hash. If `vk` is given, every signed checkpoint must also match the ledger and verify.
    pub fn verify_chain(&self, vk: Option<&VerifyingKey>) -> Result<VerifyReport> {
        let rows = self.store.scan(LEDGER, b"", usize::MAX)?;
        let mut prev = GENESIS;
        let mut n = 0u64;
        for (k, v) in &rows {
            let s = decode(v)?;
            let bad = |reason: &str| AuditError::Integrity { seq: s.meta.seq, reason: reason.into() };
            if s.meta.seq != n || k.as_slice() != seq_key(n) {
                return Err(bad("sequence gap or key mismatch (entry removed or reordered)"));
            }
            if s.prev != prev {
                return Err(bad("previous-hash link broken"));
            }
            if entry_hash(&s.prev, &s.meta, &s.sealed) != s.meta.hash {
                return Err(bad("entry content does not match its hash (modified)"));
            }
            prev = s.meta.hash;
            n += 1;
        }
        let mut verified = 0;
        if let Some(vk) = vk {
            for (k, v) in self.store.scan(CHECKPOINTS, b"", usize::MAX)? {
                let seq = u64::from_be_bytes(k.as_slice().try_into().map_err(|_| AuditError::Malformed)?);
                let hash: [u8; 32] = v.get(..32).and_then(|h| h.try_into().ok()).ok_or(AuditError::Malformed)?;
                let sig = Signature::from_slice(v.get(32..).ok_or(AuditError::Malformed)?).map_err(|_| AuditError::Malformed)?;
                let err = |reason: &str| AuditError::Integrity { seq, reason: reason.into() };
                vk.verify(&checkpoint_msg(seq, &hash), &sig).map_err(|_| err("checkpoint signature invalid"))?;
                match self.meta(seq) {
                    Ok(m) if m.hash == hash => verified += 1,
                    Ok(_) => return Err(err("checkpoint hash differs from ledger (history rewritten)")),
                    Err(_) => return Err(err("checkpointed entry missing (ledger truncated)")),
                }
            }
        }
        Ok(VerifyReport { entries: n, head_hash: if n == 0 { None } else { Some(prev) }, checkpoints_verified: verified })
    }
}

fn checkpoint_msg(seq: u64, hash: &[u8; 32]) -> Vec<u8> {
    let mut w = W(Vec::new());
    w.bytes(b"XIAR-AUDIT-CHECKPOINT-v1");
    w.u64(seq);
    w.bytes(hash);
    w.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiar_store::keyring::MasterKey;
    use xiar_store::relational::RedbStore;

    fn setup() -> (Arc<RedbStore>, Arc<Keyring>, Ledger) {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let ring = Arc::new(Keyring::new(store.clone(), MasterKey::generate()));
        let ledger = Ledger::new(store.clone(), ring.clone());
        (store, ring, ledger)
    }

    fn add(l: &Ledger, user: &str, session: &str, kind: &str, text: &str) -> EntryMeta {
        l.append(&NewEntry { session, user, bot: "bot-1", kind, tags: &["t"], payload: text.as_bytes(), ts_ms: now_ms() }).unwrap()
    }

    #[test]
    fn append_verify_and_read_back() {
        let (_s, _r, l) = setup();
        assert_eq!(l.verify_chain(None).unwrap().entries, 0);
        let a = add(&l, "u-alice", "s1", "message", "hello");
        let b = add(&l, "u-alice", "s1", "tool_call", "{\"tool\":\"x\"}");
        assert_eq!((a.seq, b.seq), (0, 1));
        let rep = l.verify_chain(None).unwrap();
        assert_eq!((rep.entries, rep.head_hash), (2, Some(b.hash)));
        assert_eq!(l.read_payload(0).unwrap(), b"hello");
        assert_eq!(l.head().unwrap().unwrap().seq, 1);
    }

    #[test]
    fn modification_deletion_and_reorder_are_detected() {
        let (s, _r, l) = setup();
        for i in 0..4 {
            add(&l, "u1", "s1", "message", &format!("m{i}"));
        }
        let original = s.get(LEDGER, &seq_key(2)).unwrap().unwrap();
        // modify metadata
        let mut st = decode(&original).unwrap();
        st.meta.kind = "forged".into();
        s.put(LEDGER, &seq_key(2), &encode(&st)).unwrap();
        assert!(matches!(l.verify_chain(None), Err(AuditError::Integrity { seq: 2, .. })));
        s.put(LEDGER, &seq_key(2), &original).unwrap();
        assert!(l.verify_chain(None).is_ok());
        // delete a middle entry
        s.delete(LEDGER, &seq_key(1)).unwrap();
        assert!(matches!(l.verify_chain(None), Err(AuditError::Integrity { .. })));
    }

    #[test]
    fn crypto_shredding_keeps_chain_valid_and_content_unreadable() {
        let (_s, ring, l) = setup();
        add(&l, "u-alice", "s1", "message", "alice secret");
        add(&l, "u-bob", "s2", "message", "bob hello");
        add(&l, "u-alice", "s1", "message", "more alice");
        ring.shred("u-alice").unwrap();
        assert!(matches!(l.read_payload(0), Err(AuditError::Store(StoreError::KeyErased(_)))));
        assert!(matches!(l.read_payload(2), Err(AuditError::Store(StoreError::KeyErased(_)))));
        assert_eq!(l.read_payload(1).unwrap(), b"bob hello");
        assert_eq!(l.verify_chain(None).unwrap().entries, 3, "hash chain still verifies after erasure");
    }

    #[test]
    fn sessions_are_indexed_and_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.redb");
        let master = MasterKey::from_bytes([7; 32]);
        let sid;
        {
            let store = Arc::new(RedbStore::open(&path).unwrap());
            let ring = Arc::new(Keyring::new(store.clone(), master.clone()));
            let l = Ledger::new(store, ring);
            sid = l.start_session("u1", "bot-1", &["support"]).unwrap();
            add(&l, "u1", &sid, "message", "hi");
            l.end_session(&sid).unwrap();
        }
        let store = Arc::new(RedbStore::open(&path).unwrap());
        let ring = Arc::new(Keyring::new(store.clone(), master));
        let l = Ledger::new(store, ring);
        let kinds: Vec<String> = l.session_entries(&sid).unwrap().into_iter().map(|m| m.kind).collect();
        assert_eq!(kinds, vec!["session_start", "message", "session_end"]);
        assert_eq!(l.verify_chain(None).unwrap().entries, 3);
        assert_eq!(l.read_payload(1).unwrap(), b"hi");
    }

    #[test]
    fn concurrent_appends_produce_a_single_valid_chain() {
        let (_s, _r, l) = setup();
        let l = Arc::new(l);
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let l = l.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        add(&l, &format!("u{t}"), &format!("s{t}"), "message", &format!("{t}-{i}"));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(l.verify_chain(None).unwrap().entries, 200);
    }

    #[test]
    fn signed_checkpoints_detect_truncation_and_rewrites() {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let ring = Arc::new(Keyring::new(store.clone(), MasterKey::generate()));
        let sk = Ledger::generate_signing_key();
        let vk = sk.verifying_key();
        let l = Ledger::new(store.clone(), ring).with_signer(sk);
        for i in 0..5 {
            add(&l, "u1", "s1", "message", &format!("m{i}"));
        }
        assert_eq!(l.checkpoint().unwrap(), Some(4));
        assert_eq!(l.verify_chain(Some(&vk)).unwrap().checkpoints_verified, 1);
        // truncate the tail: a plain chain check passes, the checkpoint check fails
        store.delete(LEDGER, &seq_key(4)).unwrap();
        assert!(l.verify_chain(None).is_ok());
        assert!(matches!(l.verify_chain(Some(&vk)), Err(AuditError::Integrity { seq: 4, .. })));
        // wrong verifying key
        let other = Ledger::generate_signing_key().verifying_key();
        assert!(l.verify_chain(Some(&other)).is_err());
    }

    #[test]
    fn payload_cannot_be_moved_to_another_user_or_kind() {
        let (s, _r, l) = setup();
        add(&l, "u-alice", "s1", "message", "alice data");
        add(&l, "u-bob", "s1", "message", "bob data");
        // swap alice's sealed payload into bob's row (and fix up nothing else): hash check catches it,
        // and even with a recomputed hash the AAD/key binding makes decryption fail.
        let a = decode(&s.get(LEDGER, &seq_key(0)).unwrap().unwrap()).unwrap();
        let mut b = decode(&s.get(LEDGER, &seq_key(1)).unwrap().unwrap()).unwrap();
        b.sealed = a.sealed.clone();
        b.meta.hash = entry_hash(&b.prev, &b.meta, &b.sealed);
        s.put(LEDGER, &seq_key(1), &encode(&b)).unwrap();
        assert!(l.read_payload(1).is_err());
    }
}
