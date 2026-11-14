// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Typed configuration (TOML). Shape follows MIRIX's `mirix.yaml` (`llm_config`, `embedding_config`,
//! `system_prompts_folder`, `meta_agent_config.agents[]`) with XIAR's additions: named LLM profiles + an
//! agent -> profile routing table (default everything to one profile), local-by-default providers, and privacy
//! invariants that cannot be switched off from a config file.
//!
//! API keys NEVER appear here: they live in the OS keychain (`secrets`). A config containing a key-like field is
//! rejected rather than silently accepted.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config: {0}")]
    Io(String),
    #[error("invalid config: {0}")]
    Parse(String),
    #[error("{0}")]
    Invalid(String),
}

/// The ten MIRIX agents plus the chat agent. Names match upstream's `*_agent` identifiers.
pub const KNOWN_AGENTS: &[&str] = &[
    "chat_agent",
    "meta_memory_agent",
    "core_memory_agent",
    "episodic_memory_agent",
    "semantic_memory_agent",
    "procedural_memory_agent",
    "resource_memory_agent",
    "knowledge_vault_memory_agent",
    "reflexion_agent",
    "background_agent",
    "auto_dream_agent",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub prompts_dir: PathBuf,
    pub scripts_dir: PathBuf,
    pub plugins_dir: PathBuf,
}

/// One way of running a generative model: which engine plug-in, which model, on which device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// engine plug-in id, e.g. "engine-mistralrs"
    pub engine: String,
    pub model: String,
    pub context_window: u32,
    /// "auto", "cpu", "gpu0", "npu", ...
    #[serde(default = "auto")]
    pub device: String,
}

fn auto() -> String {
    "auto".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Llm {
    pub profiles: BTreeMap<String, Profile>,
    /// profile used by every agent without an override
    pub default_profile: String,
    /// agent name -> profile name
    #[serde(default)]
    pub routing: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Embedding {
    /// embedding plug-in id, chosen at first run from the "best for your hardware" list
    pub plugin: String,
    pub model: String,
    pub dim: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CoreBlock {
    pub label: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MetaAgent {
    pub agents: Vec<String>,
    #[serde(default)]
    pub core_blocks: Vec<CoreBlock>,
    /// messages buffered before the accumulator hands them to the meta agent (upstream TEMPORARY_MESSAGE_LIMIT)
    #[serde(default = "default_limit")]
    pub temporary_message_limit: u32,
    /// skip the meta agent and fan out to the memory agents in parallel (upstream SKIP_META_MEMORY_MANAGER)
    #[serde(default)]
    pub skip_meta_memory_manager: bool,
}

fn default_limit() -> u32 {
    20
}

/// Fixed privacy invariants. They are parsed so a config that tries to change them is rejected, not ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Privacy {
    #[serde(default)]
    pub telemetry: bool,
    #[serde(default)]
    pub network_listener: bool,
    #[serde(default)]
    pub cloud_providers_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub paths: Paths,
    pub llm: Llm,
    pub embedding: Embedding,
    pub meta_agent: MetaAgent,
    #[serde(default)]
    pub privacy: Privacy,
}

impl Config {
    /// Sensible local-only defaults rooted at `data_dir`.
    pub fn default_for(data_dir: &Path) -> Self {
        let profile = Profile { engine: "engine-mistralrs".into(), model: "local-default".into(), context_window: 8192, device: auto() };
        Config {
            paths: Paths {
                data_dir: data_dir.to_path_buf(),
                prompts_dir: data_dir.join("prompts"),
                scripts_dir: data_dir.join("scripts"),
                plugins_dir: data_dir.join("plugins"),
            },
            llm: Llm { profiles: BTreeMap::from([("default".to_string(), profile)]), default_profile: "default".into(), routing: BTreeMap::new() },
            embedding: Embedding { plugin: "embedding-default".into(), model: "default".into(), dim: 768 },
            meta_agent: MetaAgent {
                agents: KNOWN_AGENTS.iter().filter(|a| **a != "chat_agent").map(|s| s.to_string()).collect(),
                core_blocks: vec![
                    CoreBlock { label: "human".into(), value: String::new() },
                    CoreBlock { label: "persona".into(), value: "I am a helpful assistant. I always say that I am an AI.".into() },
                ],
                temporary_message_limit: default_limit(),
                skip_meta_memory_manager: false,
            },
            privacy: Privacy::default(),
        }
    }

    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        // Reject anything that looks like a secret before even deserializing into typed fields.
        let raw: toml::Value = toml::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))?;
        reject_secrets(&raw, "")?;
        let c: Config = raw.try_into().map_err(|e: toml::de::Error| ConfigError::Parse(e.to_string()))?;
        c.validate()?;
        Ok(c)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        Self::from_toml(&std::fs::read_to_string(path).map_err(|e| ConfigError::Io(e.to_string()))?)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("config always serializes")
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let bad = |m: String| Err(ConfigError::Invalid(m));
        if self.privacy != Privacy::default() {
            return bad("privacy.telemetry / network_listener / cloud_providers_enabled cannot be enabled from a config file (cloud providers are enabled per plug-in through the opt-in flow)".into());
        }
        if !self.llm.profiles.contains_key(&self.llm.default_profile) {
            return bad(format!("llm.default_profile {:?} is not defined in llm.profiles", self.llm.default_profile));
        }
        for (agent, profile) in &self.llm.routing {
            if !KNOWN_AGENTS.contains(&agent.as_str()) {
                return bad(format!("llm.routing: unknown agent {agent:?}"));
            }
            if !self.llm.profiles.contains_key(profile) {
                return bad(format!("llm.routing: agent {agent:?} routed to undefined profile {profile:?}"));
            }
        }
        for a in &self.meta_agent.agents {
            if !KNOWN_AGENTS.contains(&a.as_str()) || a == "chat_agent" {
                return bad(format!("meta_agent.agents: {a:?} is not a memory agent"));
            }
        }
        let mut labels = std::collections::HashSet::new();
        for b in &self.meta_agent.core_blocks {
            if b.label.is_empty() || !labels.insert(&b.label) {
                return bad(format!("meta_agent.core_blocks: empty or duplicate label {:?}", b.label));
            }
            if b.value.chars().count() > 5000 {
                return bad(format!("meta_agent.core_blocks: block {:?} exceeds the 5000 character limit", b.label));
            }
        }
        if self.meta_agent.temporary_message_limit == 0 {
            return bad("meta_agent.temporary_message_limit must be at least 1".into());
        }
        if self.embedding.dim == 0 {
            return bad("embedding.dim must be positive".into());
        }
        for p in self.llm.profiles.values() {
            if p.context_window < 512 {
                return bad(format!("profile model {:?}: context_window too small", p.model));
            }
        }
        Ok(())
    }

    /// Profile an agent runs on: its routing override, else the default profile.
    pub fn profile_for(&self, agent: &str) -> &Profile {
        let name = self.llm.routing.get(agent).unwrap_or(&self.llm.default_profile);
        &self.llm.profiles[name]
    }
}

fn reject_secrets(v: &toml::Value, path: &str) -> Result<(), ConfigError> {
    const NEEDLES: &[&str] = &["api_key", "apikey", "secret", "password", "token", "private_key"];
    if let toml::Value::Table(t) = v {
        for (k, val) in t {
            let full = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
            let lk = k.to_ascii_lowercase();
            if NEEDLES.iter().any(|n| lk.contains(n)) {
                return Err(ConfigError::Invalid(format!("{full}: secrets must not be stored in the config file; they belong in the OS keychain")));
            }
            reject_secrets(val, &full)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default_for(Path::new("/data/xiar"))
    }

    #[test]
    fn default_config_is_valid_and_roundtrips_through_toml() {
        let c = cfg();
        c.validate().unwrap();
        let back = Config::from_toml(&c.to_toml()).unwrap();
        assert_eq!(c, back);
        assert!(c.meta_agent.agents.contains(&"auto_dream_agent".to_string()));
        assert!(!c.meta_agent.agents.contains(&"chat_agent".to_string()));
    }

    #[test]
    fn routing_overrides_and_default_profile() {
        let mut c = cfg();
        c.llm.profiles.insert("cpu_bg".into(), Profile { engine: "engine-mistralrs".into(), model: "small".into(), context_window: 4096, device: "cpu".into() });
        c.llm.routing.insert("auto_dream_agent".into(), "cpu_bg".into());
        c.validate().unwrap();
        assert_eq!(c.profile_for("auto_dream_agent").device, "cpu");
        assert_eq!(c.profile_for("chat_agent").device, "auto");
    }

    #[test]
    fn rejects_dangling_references_and_unknown_agents() {
        let mut c = cfg();
        c.llm.default_profile = "missing".into();
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.llm.routing.insert("auto_dream_agent".into(), "nope".into());
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.llm.routing.insert("evil_agent".into(), "default".into());
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.meta_agent.agents.push("chat_agent".into());
        assert!(c.validate().is_err());
    }

    #[test]
    fn privacy_invariants_cannot_be_changed_from_config() {
        for field in ["telemetry", "network_listener", "cloud_providers_enabled"] {
            let t = cfg().to_toml().replace("[privacy]", &format!("[privacy]\n{field} = true"));
            let t = if t.contains("[privacy]") { t } else { format!("{t}\n[privacy]\n{field} = true\n") };
            assert!(Config::from_toml(&t).is_err(), "{field} = true must be rejected");
        }
    }

    #[test]
    fn secrets_in_config_are_rejected() {
        let t = format!("{}\n[providers.openai]\napi_key = \"sk-123\"\n", cfg().to_toml());
        let e = Config::from_toml(&t).unwrap_err().to_string();
        assert!(e.contains("OS keychain"), "{e}");
        let t = cfg().to_toml().replace("model = \"local-default\"", "model = \"local-default\"\nauth_token = \"x\"");
        assert!(Config::from_toml(&t).is_err());
    }

    #[test]
    fn unknown_fields_and_bad_limits_are_rejected() {
        assert!(Config::from_toml(&format!("{}\nsurprise = 1\n", cfg().to_toml())).is_err());
        let mut c = cfg();
        c.meta_agent.temporary_message_limit = 0;
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.meta_agent.core_blocks[0].value = "x".repeat(5001);
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.meta_agent.core_blocks.push(CoreBlock { label: "human".into(), value: String::new() });
        assert!(c.validate().is_err(), "duplicate block label");
    }
}
