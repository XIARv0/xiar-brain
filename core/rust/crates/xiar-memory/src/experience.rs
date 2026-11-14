// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Skill experiences: lessons distilled from sealed sessions (`worth_learning` / `worth_avoiding`) that feed skill
//! evolution. Ported from MIRIX `skill_experience_manager`. Each record carries quotes from the user's conversations, so
//! it is sealed with the owner's key; erasure covers it like every other memory table.

use crate::manager::{MemError, Result, Sealer};
use crate::types::{now_ms, ExperienceStatus, ExperienceType, SkillExperience};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use xiar_store::relational::RelationalStore;

const BY_USER: &str = "sexp_by_user";
const BY_ID: &str = "sexp_by_id";

#[derive(Debug, Clone, PartialEq)]
pub struct Aggregate {
    pub n: usize,
    pub n_worth_learning: usize,
    pub n_worth_avoiding: usize,
    pub sum_priority: f32,
}

pub struct ExperienceManager {
    store: Arc<dyn RelationalStore>,
    sealer: Arc<dyn Sealer>,
    lock: Mutex<()>,
}

fn key(user: &str, id: &str) -> Vec<u8> {
    [user.as_bytes(), b"\0", id.as_bytes()].concat()
}

fn aad(user: &str, id: &str) -> Vec<u8> {
    [b"xiar-skill-experience:".as_slice(), user.as_bytes(), b"\0", id.as_bytes()].concat()
}

fn priority(e: &SkillExperience) -> f32 {
    e.importance * e.credibility
}

impl ExperienceManager {
    pub fn new(store: Arc<dyn RelationalStore>, sealer: Arc<dyn Sealer>) -> Self {
        Self { store, sealer, lock: Mutex::new(()) }
    }

    fn read(&self, user: &str, id: &str) -> Result<Option<SkillExperience>> {
        let Some(sealed) = self.store.get(BY_USER, &key(user, id))? else { return Ok(None) };
        let json = self.sealer.open(user, &aad(user, id), &sealed)?;
        serde_json::from_slice(&json).map(Some).map_err(|e| MemError::Serde(e.to_string()))
    }

    fn write(&self, e: &SkillExperience) -> Result<()> {
        let json = serde_json::to_vec(e).map_err(|x| MemError::Serde(x.to_string()))?;
        let sealed = self.sealer.seal(&e.user_id, &aad(&e.user_id, &e.id), &json)?;
        self.store.put(BY_USER, &key(&e.user_id, &e.id), &sealed)?;
        self.store.put(BY_ID, e.id.as_bytes(), e.user_id.as_bytes())?;
        Ok(())
    }

    fn owner_of(&self, id: &str) -> Result<Option<String>> {
        Ok(self.store.get(BY_ID, id.as_bytes())?.map(|v| String::from_utf8_lossy(&v).into_owned()))
    }

    /// Insert one experience (status `pending` unless the item says otherwise).
    pub fn create(&self, mut e: SkillExperience) -> Result<SkillExperience> {
        e.clamp_scores(); // upstream clamps rather than refuses
        e.validate()?;
        let _g = self.lock.lock().unwrap();
        self.write(&e)?;
        Ok(e)
    }

    /// Distinct agent ids that own experiences for this user.
    pub fn agent_ids(&self, user: &str) -> Result<Vec<String>> {
        let mut ids: Vec<String> = self.all_for_user(user)?.into_iter().filter_map(|e| e.agent_id).collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    pub fn get(&self, user: &str, id: &str) -> Result<Option<SkillExperience>> {
        self.read(user, id)
    }

    fn all_for_user(&self, user: &str) -> Result<Vec<SkillExperience>> {
        let prefix = [user.as_bytes(), b"\0"].concat();
        let mut out = Vec::new();
        for (k, _) in self.store.scan(BY_USER, &prefix, usize::MAX)? {
            let id = String::from_utf8_lossy(&k[prefix.len()..]).into_owned();
            if let Some(e) = self.read(user, &id)? {
                out.push(e);
            }
        }
        Ok(out)
    }

    /// An agent's experiences for a user, best first (`importance * credibility` desc, then newest).
    /// `status = None` returns every status; `ids = Some(&[])` is an explicit "scope to nothing".
    pub fn list(&self, agent_id: &str, user: &str, status: Option<ExperienceStatus>, limit: usize, ids: Option<&[String]>) -> Result<Vec<SkillExperience>> {
        if matches!(ids, Some(i) if i.is_empty()) {
            return Ok(vec![]);
        }
        let idset: Option<BTreeSet<&str>> = ids.map(|i| i.iter().map(String::as_str).collect());
        let mut v: Vec<SkillExperience> = self
            .all_for_user(user)?
            .into_iter()
            .filter(|e| e.agent_id.as_deref() == Some(agent_id))
            .filter(|e| status.is_none_or(|s| e.status == s))
            .filter(|e| idset.as_ref().is_none_or(|s| s.contains(e.id.as_str())))
            .collect();
        v.sort_by(|a, b| priority(b).partial_cmp(&priority(a)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| b.created_at_ms.cmp(&a.created_at_ms)));
        if limit > 0 {
            v.truncate(limit);
        }
        Ok(v)
    }

    fn set_status(&self, ids: &[String], status: ExperienceStatus, consumed_by: Option<&str>, influenced: Option<&[String]>, owner: Option<(&str, &str)>) -> Result<usize> {
        let _g = self.lock.lock().unwrap();
        let mut n = 0;
        for id in ids {
            let Some(user) = self.owner_of(id)? else { continue };
            let Some(mut e) = self.read(&user, id)? else { continue };
            if e.status != ExperienceStatus::Pending {
                continue; // only pending rows transition: a consumed experience is never re-flipped
            }
            if let Some((agent, u)) = owner {
                if e.agent_id.as_deref() != Some(agent) || e.user_id != u {
                    continue;
                }
            }
            e.status = status;
            if let Some(c) = consumed_by {
                e.consumed_by = Some(c.to_string());
            }
            if let Some(i) = influenced {
                e.influenced_skill_ids = Some(i.to_vec());
            }
            e.updated_at_ms = Some(now_ms());
            self.write(&e)?;
            n += 1;
        }
        Ok(n)
    }

    /// Mark pending experiences consumed by an evolution run. Idempotent; returns how many changed.
    pub fn mark_consumed(&self, ids: &[String], run_id: &str, influenced_skill_ids: Option<&[String]>) -> Result<usize> {
        self.set_status(ids, ExperienceStatus::Consumed, Some(run_id), influenced_skill_ids, None)
    }

    /// Retire still-pending experiences. `owner = Some((agent_id, user_id))` guards against foreign ids.
    pub fn mark_superseded(&self, ids: &[String], owner: Option<(&str, &str)>) -> Result<usize> {
        self.set_status(ids, ExperienceStatus::Superseded, None, None, owner)
    }

    pub fn aggregate(&self, user: &str, ids: &[String]) -> Result<Aggregate> {
        let mut a = Aggregate { n: 0, n_worth_learning: 0, n_worth_avoiding: 0, sum_priority: 0.0 };
        for id in ids {
            if let Some(e) = self.read(user, id)? {
                a.n += 1;
                a.sum_priority += priority(&e);
                match e.experience_type {
                    ExperienceType::WorthLearning => a.n_worth_learning += 1,
                    ExperienceType::WorthAvoiding => a.n_worth_avoiding += 1,
                }
            }
        }
        Ok(a)
    }

    /// Erasure: remove every experience distilled from a user's sessions, whatever its status.
    pub fn delete_user(&self, user: &str) -> Result<usize> {
        let _g = self.lock.lock().unwrap();
        let prefix = [user.as_bytes(), b"\0"].concat();
        let mut n = 0;
        for (k, _) in self.store.scan(BY_USER, &prefix, usize::MAX)? {
            let id = &k[prefix.len()..];
            self.store.delete(BY_ID, id)?;
            if self.store.delete(BY_USER, &k)? {
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

    fn mgr() -> (Arc<RedbStore>, Arc<Keyring>, ExperienceManager) {
        let store = Arc::new(RedbStore::in_memory().unwrap());
        let ring = Arc::new(Keyring::new(store.clone(), MasterKey::generate()));
        (store.clone(), ring.clone(), ExperienceManager::new(store, ring))
    }

    fn exp(user: &str, agent: &str, t: ExperienceType, title: &str, imp: f32, cred: f32) -> SkillExperience {
        let mut e = SkillExperience::new(user, "sess-1", t, title, "lesson text", imp, cred, "{\"quote\":\"user said no\"}");
        e.agent_id = Some(agent.into());
        e
    }

    #[test]
    fn list_orders_by_priority_and_filters() {
        let (_s, _r, m) = mgr();
        let low = m.create(exp("alice", "a1", ExperienceType::WorthLearning, "low", 0.2, 0.5)).unwrap();
        let high = m.create(exp("alice", "a1", ExperienceType::WorthAvoiding, "high", 0.9, 0.9)).unwrap();
        m.create(exp("alice", "a2", ExperienceType::WorthLearning, "other agent", 1.0, 1.0)).unwrap();
        m.create(exp("bob", "a1", ExperienceType::WorthLearning, "other user", 1.0, 1.0)).unwrap();
        let titles: Vec<String> = m.list("a1", "alice", Some(ExperienceStatus::Pending), 100, None).unwrap().into_iter().map(|e| e.title).collect();
        assert_eq!(titles, vec!["high", "low"]);
        assert_eq!(m.list("a1", "alice", None, 1, None).unwrap().len(), 1);
        assert!(m.list("a1", "alice", None, 100, Some(&[])).unwrap().is_empty(), "ids=[] scopes to nothing");
        let only = m.list("a1", "alice", None, 100, Some(std::slice::from_ref(&low.id))).unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].id, low.id);
        assert!(m.get("bob", &high.id).unwrap().is_none(), "other users cannot read it");
    }

    #[test]
    fn only_pending_rows_transition_and_consumed_is_never_clobbered() {
        let (_s, _r, m) = mgr();
        let a = m.create(exp("alice", "a1", ExperienceType::WorthLearning, "a", 0.5, 0.5)).unwrap();
        let b = m.create(exp("alice", "a1", ExperienceType::WorthAvoiding, "b", 0.5, 0.5)).unwrap();
        let ids = vec![a.id.clone(), b.id.clone()];
        assert_eq!(m.mark_consumed(std::slice::from_ref(&a.id), "run-1", Some(&["skill-9".to_string()])).unwrap(), 1);
        assert_eq!(m.mark_consumed(std::slice::from_ref(&a.id), "run-2", None).unwrap(), 0, "second consumer cannot clobber");
        let got = m.get("alice", &a.id).unwrap().unwrap();
        assert_eq!((got.status, got.consumed_by.as_deref(), got.influenced_skill_ids.as_deref()), (ExperienceStatus::Consumed, Some("run-1"), Some(&["skill-9".to_string()][..])));
        // supersede a mixed set: only the still-pending one is retired
        assert_eq!(m.mark_superseded(&ids, None).unwrap(), 1);
        assert_eq!(m.get("alice", &a.id).unwrap().unwrap().status, ExperienceStatus::Consumed);
        assert_eq!(m.get("alice", &b.id).unwrap().unwrap().status, ExperienceStatus::Superseded);
        assert!(m.list("a1", "alice", Some(ExperienceStatus::Pending), 10, None).unwrap().is_empty());
    }

    #[test]
    fn supersede_with_owner_guard_ignores_foreign_ids() {
        let (_s, _r, m) = mgr();
        let mine = m.create(exp("alice", "a1", ExperienceType::WorthLearning, "mine", 0.5, 0.5)).unwrap();
        let theirs = m.create(exp("bob", "a9", ExperienceType::WorthLearning, "theirs", 0.5, 0.5)).unwrap();
        let n = m.mark_superseded(&[mine.id.clone(), theirs.id.clone()], Some(("a1", "alice"))).unwrap();
        assert_eq!(n, 1);
        assert_eq!(m.get("bob", &theirs.id).unwrap().unwrap().status, ExperienceStatus::Pending);
    }

    #[test]
    fn aggregate_counts_types_and_priority() {
        let (_s, _r, m) = mgr();
        let a = m.create(exp("alice", "a1", ExperienceType::WorthLearning, "a", 1.0, 0.5)).unwrap();
        let b = m.create(exp("alice", "a1", ExperienceType::WorthAvoiding, "b", 0.5, 0.5)).unwrap();
        let agg = m.aggregate("alice", &[a.id.clone(), b.id.clone(), "sexp-missing".into()]).unwrap();
        assert_eq!((agg.n, agg.n_worth_learning, agg.n_worth_avoiding), (2, 1, 1));
        assert!((agg.sum_priority - 0.75).abs() < 1e-6);
        assert_eq!(m.aggregate("alice", &[]).unwrap().n, 0);
    }

    #[test]
    fn sealed_at_rest_validated_and_erasable() {
        let (s, ring, m) = mgr();
        let e = m.create(exp("alice", "a1", ExperienceType::WorthLearning, "t", 0.5, 0.5)).unwrap();
        let raw = s.get(BY_USER, &key("alice", &e.id)).unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("user said no"), "evidence quotes are not stored in clear");
        // scores clamp like upstream's schema; over-long text is refused
        let clamped = m.create(exp("alice", "a1", ExperienceType::WorthLearning, "t", 2.0, f32::NAN)).unwrap();
        assert_eq!((clamped.importance, clamped.credibility), (1.0, 0.0));
        let mut long = exp("alice", "a1", ExperienceType::WorthLearning, "t", 0.5, 0.5);
        long.title = "x".repeat(257);
        assert!(m.create(long).is_err());
        assert_eq!(m.delete_user("alice").unwrap(), 2);
        assert!(m.get("alice", &e.id).unwrap().is_none());
        assert!(m.owner_of(&e.id).unwrap().is_none(), "id index cleaned too");
        ring.shred("alice").unwrap();
    }
}
