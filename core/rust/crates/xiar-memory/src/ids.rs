// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Id generation. MIRIX ids are `<prefix>-<uuid4>`; raw memories use UUIDv7 (time-ordered) without a prefix.

use uuid::Uuid;

pub const EPISODIC: &str = "ep_mem";
pub const SEMANTIC: &str = "sem_item";
pub const PROCEDURAL: &str = "proc_item";
pub const RESOURCE: &str = "res_item";
pub const KNOWLEDGE_VAULT: &str = "kv_item";
pub const RAW: &str = "raw_mem";
pub const SKILL_EXPERIENCE: &str = "sexp";
pub const BLOCK: &str = "block";

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

/// Raw memories are keyed by a bare UUIDv7 so ids sort by creation time.
pub fn new_raw_id() -> String {
    Uuid::now_v7().to_string()
}

/// True if `id` looks like `<prefix>-<uuid>`.
pub fn has_prefix(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).and_then(|r| r.strip_prefix('-')).map(|u| Uuid::parse_str(u).is_ok()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixed_ids_are_unique_and_validate() {
        let a = new_id(EPISODIC);
        let b = new_id(EPISODIC);
        assert_ne!(a, b);
        assert!(has_prefix(&a, EPISODIC));
        assert!(!has_prefix(&a, SEMANTIC));
        assert!(!has_prefix("ep_mem-not-a-uuid", EPISODIC));
        assert!(!has_prefix("ep_mem", EPISODIC));
    }

    #[test]
    fn raw_ids_are_time_ordered() {
        let a = new_raw_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_raw_id();
        assert!(a < b, "uuid v7 sorts by time: {a} < {b}");
        assert!(Uuid::parse_str(&a).is_ok());
    }
}
