// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! agent -> profile -> engine routing, driven by the user-editable config.

use crate::{LlmEngine, LlmError};
use std::collections::BTreeMap;
use std::sync::Arc;
use xiar_core::config::Config;

pub struct Router {
    config: Arc<Config>,
    engines: BTreeMap<String, Arc<dyn LlmEngine>>,
}

impl Router {
    /// `engines` maps profile name -> running engine (built by the engine plug-ins from the profile's engine/model/device).
    pub fn new(config: Arc<Config>, engines: BTreeMap<String, Arc<dyn LlmEngine>>) -> Self {
        Self { config, engines }
    }

    /// Name of the profile an agent runs on (override, else the default profile).
    pub fn profile_name(&self, agent: &str) -> &str {
        self.config.llm.routing.get(agent).unwrap_or(&self.config.llm.default_profile)
    }

    pub fn engine_for(&self, agent: &str) -> Result<Arc<dyn LlmEngine>, LlmError> {
        let p = self.profile_name(agent);
        self.engines.get(p).cloned().ok_or_else(|| LlmError::NoEngine(p.to_string()))
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ScriptedEngine;
    use std::path::Path;
    use xiar_core::config::Profile;

    #[test]
    fn routes_overrides_and_defaults_and_reports_missing_engines() {
        let mut cfg = Config::default_for(Path::new("/d"));
        cfg.llm.profiles.insert("bg".into(), Profile { engine: "e".into(), model: "small".into(), context_window: 4096, device: "cpu".into() });
        cfg.llm.routing.insert("auto_dream_agent".into(), "bg".into());
        cfg.validate().unwrap();
        let main: Arc<dyn LlmEngine> = Arc::new(ScriptedEngine::new("main", 8192));
        let bg: Arc<dyn LlmEngine> = Arc::new(ScriptedEngine::new("bg", 4096));
        let r = Router::new(Arc::new(cfg.clone()), BTreeMap::from([("default".to_string(), main.clone()), ("bg".to_string(), bg.clone())]));
        assert_eq!(r.engine_for("chat_agent").unwrap().model(), "main");
        assert_eq!(r.engine_for("auto_dream_agent").unwrap().model(), "bg");
        assert_eq!(r.profile_name("episodic_memory_agent"), "default");
        let missing = Router::new(Arc::new(cfg), BTreeMap::from([("default".to_string(), main)]));
        assert_eq!(missing.engine_for("auto_dream_agent").err(), Some(LlmError::NoEngine("bg".into())));
    }
}
