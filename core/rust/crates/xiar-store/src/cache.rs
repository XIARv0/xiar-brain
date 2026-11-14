// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! In-process cache behind a trait (replaces MIRIX's Redis cache provider). Values are opaque bytes; callers
//! serialize. A cache is purely an accelerator: every read path must work with `NoCache`, and anything sensitive
//! (vault secrets, sealed blocks) must not be put in it unsealed.

use moka::sync::Cache as Moka;
use std::time::Duration;

pub trait Cache: Send + Sync {
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn put(&self, key: &str, value: Vec<u8>);
    fn invalidate(&self, key: &str);
    /// Drop every entry whose key starts with `prefix` (e.g. all cached items of an erased user).
    fn invalidate_prefix(&self, prefix: &str);
    fn clear(&self);
}

pub struct NoCache;

impl Cache for NoCache {
    fn get(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
    fn put(&self, _: &str, _: Vec<u8>) {}
    fn invalidate(&self, _: &str) {}
    fn invalidate_prefix(&self, _: &str) {}
    fn clear(&self) {}
}

pub struct MokaCache {
    inner: Moka<String, Vec<u8>>,
}

impl MokaCache {
    pub fn new(max_entries: u64, ttl: Duration) -> Self {
        Self { inner: Moka::builder().max_capacity(max_entries).time_to_live(ttl).support_invalidation_closures().build() }
    }
}

impl Cache for MokaCache {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner.get(key)
    }
    fn put(&self, key: &str, value: Vec<u8>) {
        self.inner.insert(key.to_string(), value);
    }
    fn invalidate(&self, key: &str) {
        self.inner.invalidate(key);
    }
    fn invalidate_prefix(&self, prefix: &str) {
        let p = prefix.to_string();
        let _ = self.inner.invalidate_entries_if(move |k, _| k.starts_with(&p));
    }
    fn clear(&self) {
        self.inner.invalidate_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_invalidate_and_prefix_invalidation() {
        let c = MokaCache::new(100, Duration::from_secs(60));
        c.put("user-a/ep/1", b"x".to_vec());
        c.put("user-a/ep/2", b"y".to_vec());
        c.put("user-b/ep/1", b"z".to_vec());
        assert_eq!(c.get("user-a/ep/1"), Some(b"x".to_vec()));
        c.invalidate("user-a/ep/1");
        assert_eq!(c.get("user-a/ep/1"), None);
        c.invalidate_prefix("user-a/");
        c.inner.run_pending_tasks();
        assert_eq!(c.get("user-a/ep/2"), None, "erased user's entries are dropped");
        assert_eq!(c.get("user-b/ep/1"), Some(b"z".to_vec()));
        c.clear();
        assert_eq!(c.get("user-b/ep/1"), None);
    }

    #[test]
    fn expires_and_bounds_size_and_nocache_is_inert() {
        let c = MokaCache::new(100, Duration::from_millis(50));
        c.put("k", b"v".to_vec());
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(c.get("k"), None, "ttl respected");
        let n = NoCache;
        n.put("k", b"v".to_vec());
        assert_eq!(n.get("k"), None);
    }
}
