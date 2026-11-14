// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Core memory: small always-in-context blocks ("human", "persona", ...) with a character limit.
//!
//! Ported from MIRIX `core_memory_append` / `core_memory_rewrite` (functions/function_sets/memory_tools.py) and
//! `block_manager`. The messages returned to the agent are kept word-for-word because prompts and agents react to them.
//! Block values are personal content, so each block is sealed with its owner's key before it reaches the store.
//!
//! Blocks live in a **scope** (the bot's memory scope, `""` = unscoped): the same user has separate "human"/"persona"
//! blocks per bot, which is how persona memories stay isolated.
//!
//! Concurrency: operations are read-modify-write guarded by one in-process lock (the app is single-process); the lock is
//! held across the store calls so two agents appending to the same block cannot lose an update.

use crate::manager::{MemError, Result, Sealer};
use crate::types::{Block, ValidationError, CORE_MEMORY_BLOCK_CHAR_LIMIT};
use std::sync::{Arc, Mutex};
use xiar_store::relational::RelationalStore;

const TABLE: &str = "core_blocks";

/// Result of an agent-facing block operation. `Message` is shown to the agent as the tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// success, nothing to report (MIRIX returns `None`)
    Done,
    /// success or soft failure with text for the agent
    Message(String),
}

pub struct CoreMemory {
    store: Arc<dyn RelationalStore>,
    sealer: Arc<dyn Sealer>,
    lock: Mutex<()>,
}

fn key(user: &str, scope: &str, label: &str) -> Vec<u8> {
    [user.as_bytes(), b"\0", scope.as_bytes(), b"\0", label.as_bytes()].concat()
}

fn aad(user: &str, scope: &str, label: &str) -> Vec<u8> {
    [b"xiar-core-block:".as_slice(), user.as_bytes(), b"\0", scope.as_bytes(), b"\0", label.as_bytes()].concat()
}

fn looks_like_line_prefix(content: &str) -> bool {
    // MIRIX: re.match(r"^Line \d+:", content)
    let Some(rest) = content.strip_prefix("Line ") else { return false };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with(':')
}

impl CoreMemory {
    pub fn new(store: Arc<dyn RelationalStore>, sealer: Arc<dyn Sealer>) -> Self {
        Self { store, sealer, lock: Mutex::new(()) }
    }

    fn read(&self, user: &str, scope: &str, label: &str) -> Result<Option<Block>> {
        let Some(sealed) = self.store.get(TABLE, &key(user, scope, label))? else { return Ok(None) };
        let json = self.sealer.open(user, &aad(user, scope, label), &sealed)?;
        serde_json::from_slice(&json).map(Some).map_err(|e| MemError::Serde(e.to_string()))
    }

    fn write(&self, scope: &str, b: &Block) -> Result<()> {
        let json = serde_json::to_vec(b).map_err(|e| MemError::Serde(e.to_string()))?;
        let sealed = self.sealer.seal(&b.user_id, &aad(&b.user_id, scope, &b.label), &json)?;
        self.store.put(TABLE, &key(&b.user_id, scope, &b.label), &sealed)?;
        Ok(())
    }

    /// Restore a block exactly as exported (replacing any block with the same user, scope and label).
    pub fn import(&self, b: &Block) -> Result<()> {
        b.validate()?;
        let scope = b.filter_tags.as_ref().and_then(|t| t.get("scope")).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let _g = self.lock.lock().unwrap();
        self.write(&scope, b)
    }

    /// Create any of the given blocks that do not exist yet in `scope` (never overwrites). Returns every block of the
    /// user in that scope.
    pub fn ensure_defaults(&self, user: &str, scope: &str, defaults: &[(&str, &str)]) -> Result<Vec<Block>> {
        let _g = self.lock.lock().unwrap();
        for (label, value) in defaults {
            if self.read(user, scope, label)?.is_none() {
                let mut b = Block::new(user, label, value);
                if !scope.is_empty() {
                    b.filter_tags = serde_json::json!({"scope": scope}).as_object().cloned();
                }
                b.validate()?;
                self.write(scope, &b)?;
            }
        }
        drop(_g);
        self.list(user, scope)
    }

    pub fn get(&self, user: &str, scope: &str, label: &str) -> Result<Option<Block>> {
        self.read(user, scope, label)
    }

    /// All blocks of a user in one scope, ordered by label.
    pub fn list(&self, user: &str, scope: &str) -> Result<Vec<Block>> {
        let prefix = [user.as_bytes(), b"\0", scope.as_bytes(), b"\0"].concat();
        let mut out = Vec::new();
        for (k, _) in self.store.scan(TABLE, &prefix, usize::MAX)? {
            let label = String::from_utf8_lossy(&k[prefix.len()..]).into_owned();
            if let Some(b) = self.read(user, scope, &label)? {
                out.push(b);
            }
        }
        Ok(out)
    }

    /// Every block of a user across all scopes (export).
    pub fn list_all_scopes(&self, user: &str) -> Result<Vec<Block>> {
        let prefix = [user.as_bytes(), b"\0"].concat();
        let mut out = Vec::new();
        for (k, _) in self.store.scan(TABLE, &prefix, usize::MAX)? {
            let rest = &k[prefix.len()..];
            let mut it = rest.splitn(2, |b| *b == 0);
            let scope = String::from_utf8_lossy(it.next().unwrap_or_default()).into_owned();
            let label = String::from_utf8_lossy(it.next().unwrap_or_default()).into_owned();
            if let Some(b) = self.read(user, &scope, &label)? {
                out.push(b);
            }
        }
        Ok(out)
    }

    fn require(&self, user: &str, scope: &str, label: &str) -> Result<Block> {
        self.read(user, scope, label)?.ok_or_else(|| MemError::NotFound(format!("core memory block {label:?}")))
    }

    /// `core_memory_append`: add `content` on a new line at the end of the block.
    pub fn append(&self, user: &str, scope: &str, label: &str, content: &str, actor: &str) -> Result<Outcome> {
        if looks_like_line_prefix(content) {
            return Err(MemError::Invalid("You should not include 'Line n:' (here n is a number) in the content.".into()));
        }
        let _g = self.lock.lock().unwrap();
        let mut b = self.require(user, scope, label)?;
        let current_len = b.len_chars();
        let new_value = format!("{}\n{}", b.value, content).trim().to_string();
        let new_len = new_value.chars().count();
        if new_len > b.limit {
            return Ok(Outcome::Message(format!(
                "ERROR: Cannot append - would exceed {} character limit (current: {}, adding: {}, total would be: {}). Use core_memory_rewrite to condense the '{}' block first, targeting around {} characters (~50% capacity).",
                b.limit,
                current_len,
                content.chars().count(),
                new_len,
                label,
                (b.limit as f64 * 0.5) as usize
            )));
        }
        b.value = new_value;
        b.last_updated_by_id = Some(actor.to_string());
        self.write(scope, &b)?;
        Ok(Outcome::Done)
    }

    /// `core_memory_rewrite`: replace the whole block.
    pub fn rewrite(&self, user: &str, scope: &str, label: &str, content: &str, actor: &str) -> Result<Outcome> {
        let _g = self.lock.lock().unwrap();
        let mut b = self.require(user, scope, label)?;
        let new_value = content.trim().to_string();
        let new_len = new_value.chars().count();
        if new_len > b.limit {
            return Ok(Outcome::Message(format!(
                "ERROR: Rewrite failed - new content exceeds {} character limit (provided: {} characters). Please condense further, targeting around {} characters (~50% capacity).",
                b.limit,
                new_len,
                (b.limit as f64 * 0.5) as usize
            )));
        }
        if b.value == new_value {
            return Ok(Outcome::Done);
        }
        b.value = new_value;
        b.last_updated_by_id = Some(actor.to_string());
        self.write(scope, &b)?;
        let pct = (new_len as f64 / b.limit as f64 * 100.0) as usize;
        Ok(Outcome::Message(format!("Successfully rewrote '{label}' block: {new_len}/{} characters ({pct}% full).", b.limit)))
    }

    /// Change a block's character limit. Refuses a limit below the current content.
    pub fn set_limit(&self, user: &str, scope: &str, label: &str, limit: usize) -> Result<()> {
        if limit == 0 || limit > 100 * CORE_MEMORY_BLOCK_CHAR_LIMIT {
            return Err(MemError::Invalid(format!("limit must be between 1 and {}", 100 * CORE_MEMORY_BLOCK_CHAR_LIMIT)));
        }
        let _g = self.lock.lock().unwrap();
        let mut b = self.require(user, scope, label)?;
        if b.len_chars() > limit {
            return Err(ValidationError::BlockTooLong { len: b.len_chars(), limit }.into());
        }
        b.limit = limit;
        self.write(scope, &b)
    }

    /// Erase all of a user's blocks in every scope. Returns how many were removed.
    pub fn delete_user(&self, user: &str) -> Result<usize> {
        let _g = self.lock.lock().unwrap();
        let prefix = [user.as_bytes(), b"\0"].concat();
        let mut n = 0;
        for (k, _) in self.store.scan(TABLE, &prefix, usize::MAX)? {
            if self.store.delete(TABLE, &k)? {
                n += 1;
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiar_store::keyring::{Keyring, MasterKey};
    use xiar_store::relational::RedbStore;

    const S: &str = "";

    fn mem() -> (Arc<RedbStore>, Arc<Keyring>, CoreMemory) {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let ring = Arc::new(Keyring::new(store.clone(), MasterKey::generate()));
        let m = CoreMemory::new(store.clone(), ring.clone());
        (store, ring, m)
    }

    #[test]
    fn defaults_are_created_once_and_never_overwritten() {
        let (_s, _r, m) = mem();
        let blocks = m.ensure_defaults("alice", S, &[("human", ""), ("persona", "I am a helpful assistant.")]).unwrap();
        assert_eq!(blocks.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), vec!["human", "persona"]);
        m.append("alice", S, "human", "Likes tea", "agent").unwrap();
        m.ensure_defaults("alice", S, &[("human", "SHOULD NOT REPLACE"), ("extra", "new")]).unwrap();
        assert_eq!(m.get("alice", S, "human").unwrap().unwrap().value, "Likes tea");
        assert!(m.get("alice", S, "extra").unwrap().is_some());
        assert!(m.get("bob", S, "human").unwrap().is_none(), "blocks are per user");
    }

    #[test]
    fn scopes_isolate_blocks_between_bots() {
        let (_s, _r, m) = mem();
        m.ensure_defaults("alice", "bot:a", &[("persona", "I am Bot A")]).unwrap();
        m.ensure_defaults("alice", "bot:b", &[("persona", "I am Bot B")]).unwrap();
        m.append("alice", "bot:a", "persona", "secret of A", "agent").unwrap();
        assert_eq!(m.get("alice", "bot:b", "persona").unwrap().unwrap().value, "I am Bot B", "bot B never sees what was taught to bot A");
        assert_eq!(m.get("alice", "bot:a", "persona").unwrap().unwrap().value, "I am Bot A\nsecret of A");
        assert_eq!(m.get("alice", "bot:a", "persona").unwrap().unwrap().filter_tags.unwrap()["scope"], "bot:a");
        assert_eq!(m.list("alice", "bot:b").unwrap().len(), 1);
        assert_eq!(m.list_all_scopes("alice").unwrap().len(), 2);
        assert!(m.list("alice", "bot:c").unwrap().is_empty());
        assert_eq!(m.delete_user("alice").unwrap(), 2, "erasure covers every scope");
    }

    #[test]
    fn append_joins_with_newline_and_trims_like_mirix() {
        let (_s, _r, m) = mem();
        m.ensure_defaults("alice", S, &[("human", "")]).unwrap();
        assert_eq!(m.append("alice", S, "human", "First fact", "agent").unwrap(), Outcome::Done);
        m.append("alice", S, "human", "Second fact", "agent").unwrap();
        let b = m.get("alice", S, "human").unwrap().unwrap();
        assert_eq!(b.value, "First fact\nSecond fact", "empty block does not start with a blank line");
        assert_eq!(b.last_updated_by_id.as_deref(), Some("agent"));
    }

    #[test]
    fn append_over_limit_returns_the_exact_agent_message_and_changes_nothing() {
        let (_s, _r, m) = mem();
        m.ensure_defaults("alice", S, &[("human", &"x".repeat(4990))]).unwrap();
        let out = m.append("alice", S, "human", "this is more than ten characters", "agent").unwrap();
        assert_eq!(
            out,
            Outcome::Message("ERROR: Cannot append - would exceed 5000 character limit (current: 4990, adding: 32, total would be: 5023). Use core_memory_rewrite to condense the 'human' block first, targeting around 2500 characters (~50% capacity).".into())
        );
        assert_eq!(m.get("alice", S, "human").unwrap().unwrap().value.chars().count(), 4990);
    }

    #[test]
    fn line_n_prefix_is_rejected_unknown_block_is_not_found() {
        let (_s, _r, m) = mem();
        m.ensure_defaults("alice", S, &[("human", "")]).unwrap();
        assert!(matches!(m.append("alice", S, "human", "Line 3: stuff", "a"), Err(MemError::Invalid(_))));
        assert!(m.append("alice", S, "human", "Line of sight", "a").is_ok(), "only 'Line <digits>:' is rejected");
        assert!(matches!(m.append("alice", S, "nope", "x", "a"), Err(MemError::NotFound(_))));
        assert!(matches!(m.rewrite("alice", S, "nope", "x", "a"), Err(MemError::NotFound(_))));
    }

    #[test]
    fn rewrite_messages_match_mirix() {
        let (_s, _r, m) = mem();
        m.ensure_defaults("alice", S, &[("persona", "old text")]).unwrap();
        assert_eq!(m.rewrite("alice", S, "persona", "  new text  ", "agent").unwrap(), Outcome::Message("Successfully rewrote 'persona' block: 8/5000 characters (0% full).".into()));
        assert_eq!(m.get("alice", S, "persona").unwrap().unwrap().value, "new text");
        assert_eq!(m.rewrite("alice", S, "persona", "new text", "agent").unwrap(), Outcome::Done, "unchanged content is a silent no-op");
        let too_long = "y".repeat(5001);
        assert_eq!(
            m.rewrite("alice", S, "persona", &too_long, "agent").unwrap(),
            Outcome::Message("ERROR: Rewrite failed - new content exceeds 5000 character limit (provided: 5001 characters). Please condense further, targeting around 2500 characters (~50% capacity).".into())
        );
        let half = "z".repeat(2500);
        let Outcome::Message(m2) = m.rewrite("alice", S, "persona", &half, "agent").unwrap() else { panic!() };
        assert!(m2.ends_with("2500/5000 characters (50% full)."), "{m2}");
    }

    #[test]
    fn limits_unicode_and_set_limit() {
        let (_s, _r, m) = mem();
        m.ensure_defaults("alice", S, &[("human", "héllo wörld 🌍")]).unwrap();
        assert_eq!(m.get("alice", S, "human").unwrap().unwrap().len_chars(), 13, "characters, not bytes");
        m.set_limit("alice", S, "human", 20).unwrap();
        assert!(m.append("alice", S, "human", "0123456789", "a").unwrap() != Outcome::Done, "now over the lowered limit");
        assert!(m.set_limit("alice", S, "human", 5).is_err(), "cannot lower below current content");
        assert!(m.set_limit("alice", S, "human", 0).is_err());
    }

    #[test]
    fn content_is_sealed_at_rest_and_gone_after_shredding() {
        let (s, ring, m) = mem();
        m.ensure_defaults("alice", S, &[("human", "Alice's sister is Dana; password hint: blue")]).unwrap();
        let raw = s.get(TABLE, &key("alice", S, "human")).unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Dana"), "no plaintext in the store");
        ring.shred("alice").unwrap();
        assert!(m.get("alice", S, "human").is_err(), "unreadable after shredding");
        assert_eq!(m.delete_user("alice").unwrap(), 1);
    }

    #[test]
    fn concurrent_appends_do_not_lose_updates() {
        let (_s, _r, m) = mem();
        let m = Arc::new(m);
        m.ensure_defaults("alice", S, &[("human", "")]).unwrap();
        let hs: Vec<_> = (0..8).map(|t| { let m = m.clone(); std::thread::spawn(move || { for i in 0..10 { m.append("alice", S, "human", &format!("t{t}i{i}"), "agent").unwrap(); } }) }).collect();
        for h in hs { h.join().unwrap(); }
        assert_eq!(m.get("alice", S, "human").unwrap().unwrap().value.lines().count(), 80);
    }
}
