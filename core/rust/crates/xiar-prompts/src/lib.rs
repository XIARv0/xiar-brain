// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! xiar-prompts — Markdown prompt store with hot reload.
//!
//! Resolution order (matches MIRIX's `system_prompts_folder` behaviour, extended):
//!   1. runtime override (set from code/Rhai, in memory only)
//!   2. user folder (`<user_dir>/<name>.md`, edited by the user, hot-reloaded on mtime change)
//!   3. bundled default (embedded in the binary)
//!
//! File format: optional frontmatter (`---` ... `---`, simple `key: value` lines) for human readers and
//! metadata, then the prompt body. Frontmatter is never sent to the model.
//! Variables: `{{name}}`, substituted in ONE pass, so a substituted value that itself contains `{{x}}` is
//! never re-expanded (prevents variable-injection through user/LLM-provided values). Unknown variables
//! are left as literal text so typos are visible.
//!
//! Derived from the `PromptTemplate` loader in `VR XIAR V0 version 1/crates/memory/src/prompts.rs`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("invalid prompt name {0:?} (allowed: a-z 0-9 _ - and / between segments, no '..')")]
    InvalidName(String),
    #[error("prompt {0:?} not found in runtime overrides, user folder or bundled defaults")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Where a resolved prompt came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Runtime,
    User(PathBuf),
    Bundled,
}

#[derive(Debug, Clone)]
pub struct Prompt {
    pub name: String,
    pub body: String,
    /// Frontmatter `key: value` pairs (for tooling/UI; never sent to the model).
    pub meta: BTreeMap<String, String>,
    pub source: Source,
}

impl Prompt {
    /// Substitute `{{name}}` variables in a single pass.
    pub fn render(&self, vars: &[(&str, &str)]) -> String {
        render_template(&self.body, vars)
    }
}

pub struct PromptStore {
    user_dir: Option<PathBuf>,
    bundled: HashMap<&'static str, &'static str>,
    runtime: RwLock<HashMap<String, String>>,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

/// A parsed prompt with the modification time and length it was read at.
type CacheEntry = ((SystemTime, u64), Arc<Prompt>);

impl PromptStore {
    /// `bundled` is the embedded default set: `(name, file contents)`, typically built with `include_str!`.
    pub fn new(user_dir: Option<PathBuf>, bundled: &[(&'static str, &'static str)]) -> Self {
        PromptStore {
            user_dir,
            bundled: bundled.iter().copied().collect(),
            runtime: RwLock::new(HashMap::new()),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The user prompts folder, if one is configured (sibling folders such as `scripts/` live next to it).
    pub fn user_dir(&self) -> Option<&Path> {
        self.user_dir.as_deref()
    }

    pub fn get(&self, name: &str) -> Result<Arc<Prompt>, PromptError> {
        validate_name(name)?;
        if let Some(raw) = self.runtime.read().unwrap().get(name) {
            return Ok(Arc::new(parse(name, raw, Source::Runtime)));
        }
        if let Some(dir) = &self.user_dir {
            let path = dir.join(format!("{name}.md"));
            if let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified().map(|t| (t, m.len()))) {
                if let Some((cached, p)) = self.cache.lock().unwrap().get(name) {
                    if *cached == mtime {
                        return Ok(p.clone());
                    }
                }
                // unreadable user file (e.g. mid-save) falls through to the bundled default
                if let Ok(raw) = std::fs::read_to_string(&path) {
                    let p = Arc::new(parse(name, &raw, Source::User(path)));
                    self.cache.lock().unwrap().insert(name.to_string(), (mtime, p.clone()));
                    return Ok(p);
                }
            } else {
                self.cache.lock().unwrap().remove(name);
            }
        }
        match self.bundled.get(name) {
            Some(raw) => Ok(Arc::new(parse(name, raw, Source::Bundled))),
            None => Err(PromptError::NotFound(name.to_string())),
        }
    }

    pub fn render(&self, name: &str, vars: &[(&str, &str)]) -> Result<String, PromptError> {
        Ok(self.get(name)?.render(vars))
    }

    /// Override a prompt in memory only (highest priority). Does not touch disk.
    pub fn set_runtime(&self, name: &str, raw: &str) -> Result<(), PromptError> {
        validate_name(name)?;
        self.runtime.write().unwrap().insert(name.to_string(), raw.to_string());
        Ok(())
    }

    pub fn clear_runtime(&self, name: &str) {
        self.runtime.write().unwrap().remove(name);
    }

    /// First-run: copy every bundled prompt into the user folder, never overwriting an existing file.
    /// Returns how many files were written.
    pub fn install_defaults(&self) -> Result<usize, PromptError> {
        let Some(dir) = &self.user_dir else { return Ok(0) };
        let mut written = 0;
        for (name, raw) in &self.bundled {
            let path = dir.join(format!("{name}.md"));
            if path.exists() {
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, raw)?;
            written += 1;
        }
        Ok(written)
    }

    /// Names of every prompt known to the store (bundled + user folder + runtime), sorted.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.bundled.keys().map(|s| s.to_string()).collect();
        names.extend(self.runtime.read().unwrap().keys().cloned());
        if let Some(dir) = &self.user_dir {
            collect_md(dir, dir, &mut names);
        }
        names.sort();
        names.dedup();
        names
    }
}

fn collect_md(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_md(root, &p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("md") {
            if let Ok(rel) = p.strip_prefix(root) {
                let n = rel.with_extension("").to_string_lossy().replace('\\', "/");
                if validate_name(&n).is_ok() {
                    out.push(n);
                }
            }
        }
    }
}

/// Names are lowercase path-like ids: `base/core_memory_agent`. Rejects traversal and odd characters
/// because scripts/agents may pass names.
pub fn validate_name(name: &str) -> Result<(), PromptError> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && name.split('/').all(|seg| {
            !seg.is_empty() && seg != ".." && seg.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        });
    if ok { Ok(()) } else { Err(PromptError::InvalidName(name.to_string())) }
}

fn parse(name: &str, raw: &str, source: Source) -> Prompt {
    let (meta, body) = split_frontmatter(raw);
    Prompt { name: name.to_string(), body, meta, source }
}

/// Splits optional frontmatter. Handles `\n` and `\r\n`. A file that starts with `---` but never closes
/// it is treated as having no frontmatter.
fn split_frontmatter(raw: &str) -> (BTreeMap<String, String>, String) {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().unwrap_or("");
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return (BTreeMap::new(), text.to_string());
    }
    let mut meta = BTreeMap::new();
    let mut consumed = first.len();
    for line in lines {
        consumed += line.len();
        let l = line.trim_end_matches(['\r', '\n']);
        if l == "---" {
            let body = text[consumed..].trim_start_matches(['\r', '\n']).to_string();
            return (meta, body);
        }
        if let Some((k, v)) = l.split_once(':') {
            meta.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    (BTreeMap::new(), text.to_string())
}

/// Single-pass `{{name}}` substitution. Values are inserted verbatim and never re-scanned.
pub fn render_template(body: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let key = &after[..end];
                match vars.iter().find(|(k, _)| *k == key) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push_str("{{");
                        out.push_str(key);
                        out.push_str("}}");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLED: &[(&str, &str)] = &[
        ("base/greeter", "---\nrole: test\nversion: 2\n---\nHello {{name}}."),
        ("personas/default", "You are XIAR, an AI assistant."),
    ];

    fn store(dir: Option<&Path>) -> PromptStore {
        PromptStore::new(dir.map(|d| d.to_path_buf()), BUNDLED)
    }

    #[test]
    fn bundled_default_with_frontmatter_stripped_and_meta_parsed() {
        let s = store(None);
        let p = s.get("base/greeter").unwrap();
        assert_eq!(p.source, Source::Bundled);
        assert_eq!(p.meta.get("role").map(String::as_str), Some("test"));
        assert_eq!(p.meta.get("version").map(String::as_str), Some("2"));
        assert_eq!(s.render("base/greeter", &[("name", "World")]).unwrap(), "Hello World.");
    }

    #[test]
    fn resolution_order_runtime_over_user_over_bundled() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(Some(dir.path()));
        assert_eq!(s.get("personas/default").unwrap().source, Source::Bundled);
        std::fs::create_dir_all(dir.path().join("personas")).unwrap();
        std::fs::write(dir.path().join("personas/default.md"), "user version").unwrap();
        let p = s.get("personas/default").unwrap();
        assert!(matches!(p.source, Source::User(_)));
        assert_eq!(p.body, "user version");
        s.set_runtime("personas/default", "runtime version").unwrap();
        assert_eq!(s.get("personas/default").unwrap().body, "runtime version");
        s.clear_runtime("personas/default");
        assert_eq!(s.get("personas/default").unwrap().body, "user version");
        // deleting the user file falls back to bundled again
        std::fs::remove_file(dir.path().join("personas/default.md")).unwrap();
        assert_eq!(s.get("personas/default").unwrap().source, Source::Bundled);
    }

    #[test]
    fn hot_reload_on_mtime_change() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(Some(dir.path()));
        std::fs::create_dir_all(dir.path().join("base")).unwrap();
        let path = dir.path().join("base/greeter.md");
        std::fs::write(&path, "version one").unwrap();
        assert_eq!(s.get("base/greeter").unwrap().body, "version one");
        std::thread::sleep(std::time::Duration::from_millis(1100)); // coarse mtime filesystems
        std::fs::write(&path, "version two").unwrap();
        assert_eq!(s.get("base/greeter").unwrap().body, "version two");
    }

    #[test]
    fn windows_line_endings_and_bom() {
        let (meta, body) = split_frontmatter("\u{feff}---\r\nrole: x\r\n---\r\nBody line\r\nsecond");
        assert_eq!(meta.get("role").map(String::as_str), Some("x"));
        assert_eq!(body, "Body line\r\nsecond");
    }

    #[test]
    fn unclosed_frontmatter_is_plain_text() {
        let raw = "---\nrole: x\nno closing marker";
        let (meta, body) = split_frontmatter(raw);
        assert!(meta.is_empty());
        assert_eq!(body, raw);
    }

    #[test]
    fn single_pass_substitution_does_not_reexpand_values() {
        let out = render_template("A {{x}} B {{y}}", &[("x", "{{y}}"), ("y", "SECRET")]);
        assert_eq!(out, "A {{y}} B SECRET");
    }

    #[test]
    fn unknown_and_unterminated_variables_stay_literal() {
        assert_eq!(render_template("Hi {{known}} {{unknown}} {{open", &[("known", "there")]), "Hi there {{unknown}} {{open");
    }

    #[test]
    fn rejects_path_traversal_and_bad_names() {
        let s = store(None);
        for bad in ["../secrets", "a/../b", "/abs", "a//b", "Upper", "has space", "", "a\\b", "x.md"] {
            assert!(matches!(s.get(bad), Err(PromptError::InvalidName(_))), "{bad}");
        }
        assert!(matches!(s.get("nope/missing"), Err(PromptError::NotFound(_))));
    }

    #[test]
    fn install_defaults_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(Some(dir.path()));
        std::fs::create_dir_all(dir.path().join("personas")).unwrap();
        std::fs::write(dir.path().join("personas/default.md"), "mine").unwrap();
        assert_eq!(s.install_defaults().unwrap(), 1); // only base/greeter written
        assert_eq!(std::fs::read_to_string(dir.path().join("personas/default.md")).unwrap(), "mine");
        assert!(dir.path().join("base/greeter.md").exists());
        assert_eq!(s.install_defaults().unwrap(), 0);
    }

    #[test]
    fn list_merges_all_sources() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(Some(dir.path()));
        std::fs::create_dir_all(dir.path().join("custom")).unwrap();
        std::fs::write(dir.path().join("custom/mine.md"), "x").unwrap();
        s.set_runtime("rt/only", "y").unwrap();
        assert_eq!(s.list(), vec!["base/greeter", "custom/mine", "personas/default", "rt/only"]);
    }
}
