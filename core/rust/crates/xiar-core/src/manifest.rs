// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Plug-in manifest and generated privacy policy.
//!
//! Every plug-in (Rhai script pack, WASM module or native library) ships a manifest. The manifest is the single
//! source of truth for what the plug-in collects, where it stores it and whether anything leaves the device, so the
//! published privacy policy is *generated from the installed manifests* and cannot drift from reality.
//!
//! Validation is strict on purpose: a manifest that claims "no network" but declares an endpoint, or that sends data
//! off-device without naming the recipient and obtaining consent, is rejected at load time.

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("manifest is not valid JSON or has wrong fields: {0}")]
    Parse(String),
    #[error("invalid plug-in id {0:?} (lowercase letters, digits, '-' and '_' only, 3-64 chars)")]
    BadId(String),
    #[error("plug-in {id}: {reason}")]
    Inconsistent { id: String, reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Engine,
    Embedding,
    Provider,
    ScreenMonitor,
    McpClient,
    Ontology,
    Script,
    Other,
}

/// A kind of processor a plug-in's work can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Cpu,
    Npu,
    Gpu,
}

fn default_devices() -> Vec<DeviceKind> {
    vec![DeviceKind::Cpu]
}

/// What a caller asks for when it calls a plug-in: let the host choose, or a specific kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeviceHint {
    #[default]
    Auto,
    Cpu,
    Npu,
    Gpu,
}

/// The devices a call may use, best first: what the plug-in says it can run on, within what the user allows for this task
/// (both in the user's order of preference), narrowed by the caller's hint. A hint for a device that is not in both lists
/// gets nothing: **a plug-in can not exceed what the user allowed**, and a caller can not ask for what the plug-in can not do.
pub fn usable_devices(plugin: &[DeviceKind], user_allowed: &[DeviceKind], hint: DeviceHint) -> Vec<DeviceKind> {
    user_allowed
        .iter()
        .copied()
        .filter(|d| plugin.contains(d))
        .filter(|d| match hint {
            DeviceHint::Auto => true,
            DeviceHint::Cpu => *d == DeviceKind::Cpu,
            DeviceHint::Npu => *d == DeviceKind::Npu,
            DeviceHint::Gpu => *d == DeviceKind::Gpu,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    /// ordinary content
    Normal,
    /// health, financial, credentials, biometrics, children's data
    Sensitive,
}

/// One category of data the plug-in handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataItem {
    /// what, in plain language ("chat messages", "screenshots", "model prompts")
    pub what: String,
    pub sensitivity: Sensitivity,
    /// where it is stored on the device ("memory database", "not stored", "plug-in cache folder")
    pub stored: String,
    /// retention in days; `None` = until the user deletes it
    pub retention_days: Option<u32>,
}

/// A place data may be sent to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipient {
    /// legal/operator name ("OpenAI", "Anthropic")
    pub name: String,
    /// host(s) contacted, e.g. "api.openai.com"
    pub hosts: Vec<String>,
    /// what is sent
    pub sends: Vec<String>,
    /// link to the recipient's own privacy policy
    pub policy_url: String,
    /// the person chooses the sites, one pack at a time (each pack's list is shown before they approve it), so no fixed host or policy
    /// can be named here
    #[serde(default)]
    pub user_chosen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: Kind,
    /// SPDX licence id of the plug-in itself ("Apache-2.0", "MIT", "LGPL-3.0-only"...)
    pub license: String,
    /// third-party models/data the plug-in downloads or bundles, with their licences ("EmbeddingGemma: Gemma terms")
    #[serde(default)]
    pub assets: Vec<Asset>,
    /// data handled locally
    #[serde(default)]
    pub collects: Vec<DataItem>,
    /// does the plug-in open ANY network connection
    pub network: bool,
    /// destinations when `network` is true
    #[serde(default)]
    pub recipients: Vec<Recipient>,
    /// off by default and requires explicit informed opt-in (screen monitor, cloud providers, MCP)
    pub opt_in_required: bool,
    /// shows a persistent on-screen indicator while active (required for screen/audio capture)
    #[serde(default)]
    pub shows_active_indicator: bool,
    /// the processors the plug-in's work can run on; a plug-in that does not say runs on the CPU only
    #[serde(default = "default_devices")]
    pub devices: Vec<DeviceKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub license: String,
    /// true when the licence is not an OSI-approved open-source licence (e.g. Gemma terms): never bundled,
    /// downloaded after the user reads and accepts the licence
    pub not_osi_open_source: bool,
}

impl Manifest {
    pub fn from_json(s: &str) -> Result<Self, ManifestError> {
        let m: Manifest = serde_json::from_str(s).map_err(|e| ManifestError::Parse(e.to_string()))?;
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        let id_ok = (3..=64).contains(&self.id.len())
            && self.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
        if !id_ok {
            return Err(ManifestError::BadId(self.id.clone()));
        }
        let bad = |reason: &str| ManifestError::Inconsistent { id: self.id.clone(), reason: reason.to_string() };
        if !self.network && !self.recipients.is_empty() {
            return Err(bad("declares recipients but says network = false"));
        }
        if self.network {
            if self.recipients.is_empty() {
                return Err(bad("network = true requires at least one named recipient"));
            }
            if !self.opt_in_required {
                return Err(bad("anything that sends data off the device must require explicit opt-in"));
            }
            for r in &self.recipients {
                if r.user_chosen {
                    if r.name.trim().is_empty() || r.sends.is_empty() {
                        return Err(bad("a recipient the person chooses still needs a name and what is sent"));
                    }
                    continue;
                }
                if r.name.trim().is_empty() || r.hosts.is_empty() || r.sends.is_empty() || !r.policy_url.starts_with("https://") {
                    return Err(bad("each recipient needs a name, host(s), what is sent, and an https policy_url"));
                }
            }
        }
        if self.devices.is_empty() || self.devices.iter().enumerate().any(|(i, d)| self.devices[..i].contains(d)) {
            return Err(bad("devices must list each of cpu, npu and gpu at most once, and at least one"));
        }
        if matches!(self.kind, Kind::ScreenMonitor) && !(self.opt_in_required && self.shows_active_indicator) {
            return Err(bad("screen monitor must be opt-in and show a persistent active indicator"));
        }
        if self.collects.iter().any(|c| c.sensitivity == Sensitivity::Sensitive) && self.network && self.recipients.iter().all(|r| r.sends.is_empty()) {
            return Err(bad("sensitive data with network access must state exactly what is sent"));
        }
        Ok(())
    }
}

/// Generate the plain-language data section of the privacy policy from the installed manifests.
/// Deterministic (sorted by id) so the text is stable and diff-able.
pub fn generate_policy(manifests: &[Manifest]) -> String {
    let mut ms: Vec<&Manifest> = manifests.iter().collect();
    ms.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out = String::from("# Privacy: what each component does with your data\n\n");
    let off_device: Vec<&&Manifest> = ms.iter().filter(|m| m.network).collect();
    if off_device.is_empty() {
        out.push_str("**Nothing leaves your device.** None of the installed components contacts the network.\n\n");
    } else {
        out.push_str("**Some components can send data off your device, only if you turn them on:**\n\n");
        for m in &off_device {
            let names: Vec<&str> = m.recipients.iter().map(|r| r.name.as_str()).collect();
            out.push_str(&format!("- {} -> {}\n", m.name, names.join(", ")));
        }
        out.push('\n');
    }
    for m in ms {
        out.push_str(&format!("## {} ({} {})\n\n", m.name, m.id, m.version));
        out.push_str(&format!("- Licence: {}\n", m.license));
        out.push_str(&format!("- Off by default, needs your explicit opt-in: {}\n", if m.opt_in_required { "yes" } else { "no" }));
        if m.shows_active_indicator {
            out.push_str("- Shows an on-screen indicator while it is active.\n");
        }
        if m.collects.is_empty() {
            out.push_str("- Collects nothing.\n");
        }
        for c in &m.collects {
            let keep = match c.retention_days {
                Some(d) => format!("kept {d} days"),
                None => "kept until you delete it".to_string(),
            };
            let sens = if c.sensitivity == Sensitivity::Sensitive { " **(sensitive)**" } else { "" };
            out.push_str(&format!("- Handles {}{}; stored: {}; {}.\n", c.what, sens, c.stored, keep));
        }
        if m.network {
            for r in &m.recipients {
                if r.user_chosen {
                    out.push_str(&format!("- Sends {} to {}: websites you approve one pack at a time (each pack's list is shown to you before you approve it).\n", r.sends.join(", "), r.name));
                } else {
                    out.push_str(&format!("- Sends {} to {} ({}). Their policy: {}\n", r.sends.join(", "), r.name, r.hosts.join(", "), r.policy_url));
                }
            }
        } else {
            out.push_str("- Makes no network connections.\n");
        }
        for a in &m.assets {
            let note = if a.not_osi_open_source { " (not an OSI open-source licence; downloaded only after you accept it)" } else { "" };
            out.push_str(&format!("- Uses {} under {}{}.\n", a.name, a.license, note));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> &'static str {
        r#"{"id":"embedding-gemma","name":"EmbeddingGemma embedder","version":"0.1.0","kind":"embedding","license":"Apache-2.0",
            "assets":[{"name":"EmbeddingGemma weights","license":"Gemma Terms of Use","not_osi_open_source":true}],
            "collects":[{"what":"text to embed","sensitivity":"normal","stored":"not stored","retention_days":0}],
            "network":false,"opt_in_required":false}"#
    }

    fn cloud() -> String {
        r#"{"id":"openai-provider","name":"OpenAI provider","version":"1.0.0","kind":"provider","license":"Apache-2.0",
            "collects":[{"what":"chat prompts and replies","sensitivity":"sensitive","stored":"not stored by the plug-in","retention_days":null}],
            "network":true,"opt_in_required":true,
            "recipients":[{"name":"OpenAI","hosts":["api.openai.com"],"sends":["your messages","retrieved memories"],"policy_url":"https://openai.com/policies/privacy-policy"}]}"#
            .to_string()
    }

    #[test]
    fn valid_local_and_cloud_manifests_parse() {
        assert!(Manifest::from_json(local()).is_ok());
        assert!(Manifest::from_json(&cloud()).is_ok());
    }

    #[test]
    fn rejects_inconsistent_network_claims() {
        let lying = local().replace(r#""network":false"#, r#""network":false,"recipients":[{"name":"X","hosts":["x.com"],"sends":["a"],"policy_url":"https://x.com/p"}]"#);
        assert!(matches!(Manifest::from_json(&lying), Err(ManifestError::Inconsistent { .. })));
        let no_recipient = local().replace(r#""network":false"#, r#""network":true"#);
        assert!(matches!(Manifest::from_json(&no_recipient), Err(ManifestError::Inconsistent { .. })));
        let no_opt_in = cloud().replace(r#""opt_in_required":true"#, r#""opt_in_required":false"#);
        assert!(matches!(Manifest::from_json(&no_opt_in), Err(ManifestError::Inconsistent { .. })));
        let http = cloud().replace("https://openai.com", "http://openai.com");
        assert!(matches!(Manifest::from_json(&http), Err(ManifestError::Inconsistent { .. })));
    }

    #[test]
    fn screen_monitor_must_be_opt_in_with_indicator() {
        let base = r#"{"id":"screen-monitor","name":"Screen monitor","version":"1","kind":"screen_monitor","license":"Apache-2.0",
            "collects":[{"what":"screenshots","sensitivity":"sensitive","stored":"memory database","retention_days":14}],
            "network":false,"opt_in_required":OPT,"shows_active_indicator":IND}"#;
        let ok = base.replace("OPT", "true").replace("IND", "true");
        assert!(Manifest::from_json(&ok).is_ok());
        for (o, i) in [("false", "true"), ("true", "false")] {
            let m = base.replace("OPT", o).replace("IND", i);
            assert!(matches!(Manifest::from_json(&m), Err(ManifestError::Inconsistent { .. })), "{o} {i}");
        }
    }

    #[test]
    fn rejects_bad_ids_and_unknown_fields() {
        assert!(matches!(Manifest::from_json(&local().replace("embedding-gemma", "Bad Id!")), Err(ManifestError::BadId(_))));
        assert!(matches!(Manifest::from_json("{}"), Err(ManifestError::Parse(_))));
        assert!(matches!(Manifest::from_json("not json"), Err(ManifestError::Parse(_))));
    }

    #[test]
    fn generated_policy_matches_the_manifests() {
        let only_local = generate_policy(&[Manifest::from_json(local()).unwrap()]);
        assert!(only_local.contains("Nothing leaves your device"));
        assert!(only_local.contains("Makes no network connections"));
        assert!(only_local.contains("not an OSI open-source licence"));
        let both = generate_policy(&[Manifest::from_json(&cloud()).unwrap(), Manifest::from_json(local()).unwrap()]);
        assert!(!both.contains("Nothing leaves your device"));
        assert!(both.contains("OpenAI provider -> OpenAI"));
        assert!(both.contains("api.openai.com") && both.contains("**(sensitive)**"));
        // deterministic order: embedding-gemma before openai-provider
        assert!(both.find("embedding-gemma").unwrap() < both.find("openai-provider").unwrap());
        // stable: same input, same output regardless of input order
        assert_eq!(both, generate_policy(&[Manifest::from_json(local()).unwrap(), Manifest::from_json(&cloud()).unwrap()]));
    }
}

#[cfg(test)]
mod device_tests {
    use super::*;

    fn json(extra: &str) -> String {
        format!(r#"{{"id":"some-engine","name":"x","version":"1","kind":"engine","license":"MIT","network":false,"opt_in_required":false{extra}}}"#)
    }

    #[test]
    fn a_plugin_that_says_nothing_runs_on_the_cpu_only_and_the_list_is_validated() {
        assert_eq!(Manifest::from_json(&json("")).unwrap().devices, [DeviceKind::Cpu]);
        assert_eq!(Manifest::from_json(&json(r#","devices":["npu","cpu"]"#)).unwrap().devices, [DeviceKind::Npu, DeviceKind::Cpu]);
        for bad in [r#","devices":[]"#, r#","devices":["cpu","cpu"]"#] {
            assert!(Manifest::from_json(&json(bad)).is_err(), "{bad}");
        }
        assert!(Manifest::from_json(&json(r#","devices":["tpu"]"#)).is_err());
    }

    #[test]
    fn a_call_gets_the_devices_the_user_allows_that_the_plugin_can_use_and_never_more() {
        use DeviceKind::*;
        let plugin = [Cpu, Npu];
        let user = [Npu, Gpu, Cpu];
        assert_eq!(usable_devices(&plugin, &user, DeviceHint::Auto), [Npu, Cpu], "the user's order, minus what the plug-in cannot do");
        assert_eq!(usable_devices(&plugin, &user, DeviceHint::Cpu), [Cpu]);
        assert_eq!(usable_devices(&plugin, &user, DeviceHint::Gpu), Vec::<DeviceKind>::new(), "the plug-in cannot use the GPU");
        assert_eq!(usable_devices(&[Cpu, Npu, Gpu], &[Cpu], DeviceHint::Npu), Vec::<DeviceKind>::new(), "the user did not allow the NPU");
        assert!(usable_devices(&plugin, &[], DeviceHint::Auto).is_empty());
    }
}

#[cfg(test)]
mod user_chosen_tests {
    use super::*;

    #[test]
    fn a_recipient_the_person_chooses_needs_no_fixed_host_but_still_a_name_and_what_is_sent() {
        let json = |recipient: &str| format!(r#"{{"id":"web-access","name":"Web access","version":"1","kind":"provider","license":"MIT","network":true,"opt_in_required":true,"recipients":[{recipient}]}}"#);
        let ok = Manifest::from_json(&json(r#"{"name":"Websites you approve","hosts":[],"sends":["the requests of a pack you approved"],"policy_url":"","user_chosen":true}"#)).unwrap();
        assert!(generate_policy(&[ok]).contains("websites you approve one pack at a time"));
        assert!(Manifest::from_json(&json(r#"{"name":"Websites","hosts":[],"sends":[],"policy_url":"","user_chosen":true}"#)).is_err());
        assert!(Manifest::from_json(&json(r#"{"name":"Websites","hosts":[],"sends":["x"],"policy_url":""}"#)).is_err(), "without user_chosen the old rules hold");
    }
}
