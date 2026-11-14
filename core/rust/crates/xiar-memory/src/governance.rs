// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! MIRIX manager governance and audit hooks.
//!
//! Managers are callable by XIAR; every promote / forget / cross-biz proposal /
//! vault erase records a ledger entry. Deny paths are explicit.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// What kind of governed action was attempted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedAction {
    PromoteMemory,
    ForgetMemory,
    CrossBusinessProposal,
    VaultErase,
    NamespaceWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub at_ms: i64,
    pub actor: String,
    pub action: GovernedAction,
    pub decision: Decision,
    pub subject: String,
    pub detail: String,
}

/// In-memory audit ledger (host persists via xiar-audit when wired).
#[derive(Default)]
pub struct GovernanceLedger {
    entries: Mutex<Vec<LedgerEntry>>,
}

impl GovernanceLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, entry: LedgerEntry) {
        self.entries.lock().expect("ledger lock").push(entry);
    }

    pub fn entries(&self) -> Vec<LedgerEntry> {
        self.entries.lock().expect("ledger lock").clone()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().expect("ledger lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Policy: who may do what. Default — only actor `xiar` (or configured managers) may promote/forget/propose.
#[derive(Debug, Clone)]
pub struct GovernancePolicy {
    pub managers: Vec<String>,
    /// If true, cross-business proposals are allowed but always logged (never silent writes).
    pub allow_cross_business_proposals: bool,
}

impl Default for GovernancePolicy {
    fn default() -> Self {
        Self {
            managers: vec!["xiar".into(), "XIAR".into()],
            allow_cross_business_proposals: true,
        }
    }
}

impl GovernancePolicy {
    pub fn is_manager(&self, actor: &str) -> bool {
        self.managers.iter().any(|m| m.eq_ignore_ascii_case(actor))
    }
}

pub struct MirixGovernor {
    pub policy: GovernancePolicy,
    pub ledger: Arc<GovernanceLedger>,
    clock_ms: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl MirixGovernor {
    pub fn new(policy: GovernancePolicy, ledger: Arc<GovernanceLedger>) -> Self {
        Self {
            policy,
            ledger,
            clock_ms: Box::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0)
            }),
        }
    }

    fn decide(&self, actor: &str, action: GovernedAction, subject: &str, detail: &str, allow: bool) -> Decision {
        let decision = if allow { Decision::Allow } else { Decision::Deny };
        self.ledger.record(LedgerEntry {
            at_ms: (self.clock_ms)(),
            actor: actor.into(),
            action,
            decision: decision.clone(),
            subject: subject.into(),
            detail: detail.into(),
        });
        decision
    }

    pub fn promote_memory(&self, actor: &str, memory_id: &str) -> Decision {
        self.decide(actor, GovernedAction::PromoteMemory, memory_id, "promote", self.policy.is_manager(actor))
    }

    pub fn forget_memory(&self, actor: &str, memory_id: &str) -> Decision {
        self.decide(actor, GovernedAction::ForgetMemory, memory_id, "forget", self.policy.is_manager(actor))
    }

    /// Cross-biz proposals: managers only; never a silent write — always ledgered.
    pub fn cross_business_proposal(&self, actor: &str, from_scope: &str, to_scope: &str, summary: &str) -> Decision {
        let allow = self.policy.is_manager(actor) && self.policy.allow_cross_business_proposals;
        self.decide(
            actor,
            GovernedAction::CrossBusinessProposal,
            &format!("{from_scope}->{to_scope}"),
            summary,
            allow,
        )
    }

    pub fn vault_erase(&self, actor: &str, vault_id: &str) -> Decision {
        self.decide(actor, GovernedAction::VaultErase, vault_id, "vault_erase", self.policy.is_manager(actor))
    }

    pub fn namespace_write(&self, actor: &str, scope: &str) -> Decision {
        self.decide(actor, GovernedAction::NamespaceWrite, scope, "write", self.policy.is_manager(actor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_allows_promote_and_non_manager_denied() {
        let led = Arc::new(GovernanceLedger::new());
        let g = MirixGovernor::new(GovernancePolicy::default(), led.clone());
        assert_eq!(g.promote_memory("xiar", "sem-1"), Decision::Allow);
        assert_eq!(g.promote_memory("intern", "sem-1"), Decision::Deny);
        assert_eq!(led.len(), 2);
        assert_eq!(led.entries()[1].decision, Decision::Deny);
    }

    #[test]
    fn cross_business_proposal_always_ledgered() {
        let led = Arc::new(GovernanceLedger::new());
        let g = MirixGovernor::new(GovernancePolicy::default(), led.clone());
        assert_eq!(
            g.cross_business_proposal("xiar", "business:a", "business:b", "shared vendor insight"),
            Decision::Allow
        );
        assert_eq!(
            g.cross_business_proposal("bot-a", "business:a", "business:b", "sneaky"),
            Decision::Deny
        );
        let e = led.entries();
        assert_eq!(e.len(), 2);
        assert!(e.iter().all(|x| matches!(x.action, GovernedAction::CrossBusinessProposal)));
    }

    #[test]
    fn vault_erase_auditable() {
        let led = Arc::new(GovernanceLedger::new());
        let g = MirixGovernor::new(GovernancePolicy::default(), led.clone());
        assert_eq!(g.vault_erase("XIAR", "kv-9"), Decision::Allow);
        assert_eq!(led.entries()[0].action, GovernedAction::VaultErase);
        assert_eq!(led.entries()[0].subject, "kv-9");
    }

    #[test]
    fn forget_deny_path() {
        let led = Arc::new(GovernanceLedger::new());
        let g = MirixGovernor::new(GovernancePolicy::default(), led);
        assert_eq!(g.forget_memory("stranger", "ep-1"), Decision::Deny);
    }
}
