// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! `MemoryHub`: every memory layer behind one handle, plus the data-subject tools (export and erase).
//!
//! Erasure order matters: vector-store items are deleted and compacted first (real deletion), relational blocks and
//! experiences next, and the user's encryption key is destroyed last, so any sealed copy that lives elsewhere (e.g. in
//! the audit ledger, which must stay append-only) becomes unrecoverable too. The caller records the erasure in the
//! audit ledger using the returned report.

use crate::core::CoreMemory;
use crate::experience::ExperienceManager;
use crate::manager::{MemError, MemoryManager, MemoryRecord, Result, Sealer};
use crate::sources::{SourceManager, SourceRecord};
use crate::types::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use xiar_store::keyring::Keyring;
use xiar_store::relational::RelationalStore;
use xiar_store::vector::VectorStore;

/// How an import re-embeds items: `embed` returns one vector per text (or `None` on failure, in which case the item is
/// stored text-only and BM25 still finds it), `meta` describes the model that made them.
pub struct ReEmbedder<'a> {
    pub embed: &'a dyn Fn(&[String]) -> Option<Vec<Vec<f32>>>,
    pub meta: EmbeddingMeta,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub imported: usize,
    pub embedded: usize,
    /// items whose secret was redacted in the export; they cannot be restored and are skipped
    pub skipped_redacted: usize,
    pub failed: Vec<String>,
}

fn with_embeddings<T: crate::manager::MemoryRecord>(item: T, kind: MemoryKind, re: Option<&ReEmbedder<'_>>) -> (T, std::collections::BTreeMap<String, Vec<f32>>, bool) {
    let none = std::collections::BTreeMap::new();
    let Some(re) = re else { return (item, none, false) };
    let Ok(mut v) = serde_json::to_value(&item) else { return (item, none, false) };
    let fields: Vec<&str> = kind.vector_fields().iter().copied().filter(|f| v.get(*f).and_then(|x| x.as_str()).is_some_and(|t| !t.trim().is_empty())).collect();
    if fields.is_empty() {
        return (item, none, false);
    }
    let texts: Vec<String> = fields.iter().map(|f| v[*f].as_str().unwrap_or("").to_string()).collect();
    match (re.embed)(&texts) {
        Some(vs) if vs.len() == fields.len() => {
            v["embedding"] = serde_json::to_value(&re.meta).unwrap_or(serde_json::Value::Null);
            let item = serde_json::from_value(v).unwrap_or(item);
            (item, fields.iter().zip(vs).map(|(f, e)| ((*f).to_string(), e)).collect(), true)
        }
        _ => (item, none, false),
    }
}

pub struct MemoryHub {
    pub episodic: MemoryManager<EpisodicEvent>,
    pub semantic: MemoryManager<SemanticItem>,
    pub procedural: MemoryManager<ProceduralItem>,
    pub resource: MemoryManager<ResourceItem>,
    pub vault: MemoryManager<VaultItem>,
    pub raw: MemoryManager<RawMemory>,
    pub core: CoreMemory,
    pub experience: Arc<ExperienceManager>,
    pub sources: Arc<SourceManager>,
    vectors: Arc<dyn VectorStore>,
    keyring: Arc<Keyring>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErasureReport {
    pub user: String,
    pub episodic: usize,
    pub semantic: usize,
    pub procedural: usize,
    pub resource: usize,
    pub knowledge_vault: usize,
    pub raw: usize,
    pub core_blocks: usize,
    pub skill_experiences: usize,
    pub sources: usize,
    pub key_destroyed: bool,
}

impl ErasureReport {
    pub fn total_items(&self) -> usize {
        self.episodic + self.semantic + self.procedural + self.resource + self.knowledge_vault + self.raw + self.core_blocks + self.skill_experiences + self.sources
    }
}

/// Everything held about one user, in an open format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserExport {
    pub format: String,
    pub user: String,
    pub exported_at_ms: Ms,
    pub core_blocks: Vec<Block>,
    pub episodic: Vec<EpisodicEvent>,
    pub semantic: Vec<SemanticItem>,
    pub procedural: Vec<ProceduralItem>,
    pub resource: Vec<ResourceItem>,
    /// secrets are replaced by `<redacted>` unless the export was requested with `include_secrets`
    pub knowledge_vault: Vec<serde_json::Value>,
    pub raw: Vec<RawMemory>,
    pub skill_experiences: Vec<SkillExperience>,
    /// where memories came from (older exports have none)
    #[serde(default)]
    pub sources: Vec<SourceRecord>,
}

impl UserExport {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("export serializes")
    }

    /// Human-readable summary (titles only; use the JSON for full content).
    pub fn to_markdown(&self) -> String {
        let mut s = format!("# Data export for {}\n\nExported at (unix ms): {}\n\n", self.user, self.exported_at_ms);
        let mut section = |title: &str, lines: Vec<String>| {
            s.push_str(&format!("## {title} ({})\n\n", lines.len()));
            for l in lines {
                s.push_str(&format!("- {l}\n"));
            }
            s.push('\n');
        };
        section("Core memory", self.core_blocks.iter().map(|b| format!("**{}**: {}", b.label, b.value.replace('\n', " / "))).collect());
        section("Episodic memory", self.episodic.iter().map(|e| format!("{} - {}", e.event_type, e.summary)).collect());
        section("Semantic memory", self.semantic.iter().map(|e| format!("{}: {}", e.name, e.summary)).collect());
        section("Procedural memory (skills)", self.procedural.iter().map(|e| format!("{} ({}) - {}", e.name, e.entry_type, e.description)).collect());
        section("Resource memory", self.resource.iter().map(|e| format!("{} [{}] - {}", e.title, e.resource_type, e.summary)).collect());
        section("Knowledge vault", self.knowledge_vault.iter().map(|v| v.get("caption").and_then(|c| c.as_str()).unwrap_or("").to_string()).collect());
        section("Raw memory", self.raw.iter().map(|e| e.context.chars().take(80).collect::<String>()).collect());
        section("Sources", self.sources.iter().map(|r| format!("{} ({:?}){}", r.short(), r.kind, r.label.as_deref().map(|l| format!(" - {l}")).unwrap_or_default())).collect());
        section("Skill experiences", self.skill_experiences.iter().map(|e| format!("{:?}: {}", e.experience_type, e.title)).collect());
        s
    }
}

impl MemoryHub {
    /// `embedding_dim`: dimension of the configured embedding model, or `None` for text-only (BM25) mode.
    pub fn new(vectors: Arc<dyn VectorStore>, relational: Arc<dyn RelationalStore>, keyring: Arc<Keyring>, embedding_dim: Option<u32>) -> Result<Self> {
        let sealer: Arc<dyn Sealer> = keyring.clone();
        let dims = |k: MemoryKind| -> Vec<(&'static str, u32)> { embedding_dim.map(|d| k.vector_fields().iter().map(|f| (*f, d)).collect()).unwrap_or_default() };
        Ok(Self {
            episodic: MemoryManager::new(vectors.clone(), sealer.clone(), &dims(MemoryKind::Episodic))?,
            semantic: MemoryManager::new(vectors.clone(), sealer.clone(), &dims(MemoryKind::Semantic))?,
            procedural: MemoryManager::new(vectors.clone(), sealer.clone(), &dims(MemoryKind::Procedural))?,
            resource: MemoryManager::new(vectors.clone(), sealer.clone(), &dims(MemoryKind::Resource))?,
            vault: MemoryManager::new(vectors.clone(), sealer.clone(), &dims(MemoryKind::KnowledgeVault))?,
            raw: MemoryManager::new(vectors.clone(), sealer.clone(), &dims(MemoryKind::Raw))?,
            core: CoreMemory::new(relational.clone(), sealer.clone()),
            sources: Arc::new(SourceManager::new(relational.clone(), sealer.clone())),
            experience: Arc::new(ExperienceManager::new(relational, sealer)),
            vectors,
            keyring,
        })
    }

    /// Everything stored about `user`. Vault secrets are included only when `include_secrets` is true.
    pub fn export_user(&self, user: &str, include_secrets: bool) -> Result<UserExport> {
        let vault: Vec<serde_json::Value> = self
            .vault
            .list_recent(user, 0, None, None)?
            .into_iter()
            .map(|mut v| {
                if !include_secrets {
                    v.secret_value = "<redacted>".into();
                }
                serde_json::to_value(&v).map_err(|e| MemError::Serde(e.to_string()))
            })
            .collect::<Result<_>>()?;
        let mut exp = Vec::new();
        for agent in self.agent_ids_with_experiences(user)? {
            exp.extend(self.experience.list(&agent, user, None, 0, None)?);
        }
        Ok(UserExport {
            format: "xiar-user-export-v1".into(),
            user: user.to_string(),
            exported_at_ms: now_ms(),
            core_blocks: self.core.list_all_scopes(user)?,
            episodic: self.episodic.list_recent(user, 0, None, None)?,
            semantic: self.semantic.list_recent(user, 0, None, None)?,
            procedural: self.procedural.list_recent(user, 0, None, None)?,
            resource: self.resource.list_recent(user, 0, None, None)?,
            knowledge_vault: vault,
            raw: self.raw.list_recent(user, 0, None, None)?,
            skill_experiences: exp,
            sources: self.sources.list_recent(user, 0)?,
        })
    }

    /// Restore an export (a backup) for `exp.user`. Items keep their ids, so importing the same export twice replaces
    /// rather than duplicates. Embeddings are not part of an export; they are recomputed with `re` when given.
    /// A failing item is reported and does not stop the rest.
    pub fn import_user(&self, exp: &UserExport, re: Option<&ReEmbedder<'_>>) -> Result<ImportReport> {
        if exp.format != "xiar-user-export-v1" {
            return Err(MemError::Serde(format!("unsupported export format {:?}", exp.format)));
        }
        let mut r = ImportReport::default();
        macro_rules! restore {
            ($mgr:expr, $items:expr, $kind:expr) => {
                for it in $items {
                    if it.user_id() != exp.user {
                        r.failed.push(format!("{}: belongs to another user", it.id()));
                        continue;
                    }
                    let id = it.id().to_string();
                    let (it, emb, done) = with_embeddings(it.clone(), $kind, re);
                    match $mgr.insert(it, emb) {
                        Ok(_) => {
                            r.imported += 1;
                            r.embedded += usize::from(done);
                        }
                        Err(e) => r.failed.push(format!("{id}: {e}")),
                    }
                }
            };
        }
        restore!(self.episodic, &exp.episodic, MemoryKind::Episodic);
        restore!(self.semantic, &exp.semantic, MemoryKind::Semantic);
        restore!(self.procedural, &exp.procedural, MemoryKind::Procedural);
        restore!(self.resource, &exp.resource, MemoryKind::Resource);
        restore!(self.raw, &exp.raw, MemoryKind::Raw);
        for v in &exp.knowledge_vault {
            match serde_json::from_value::<VaultItem>(v.clone()) {
                Ok(it) if it.secret_value == "<redacted>" => r.skipped_redacted += 1,
                Ok(it) if it.user_id() != exp.user => r.failed.push(format!("{}: belongs to another user", it.id())),
                Ok(it) => {
                    let id = it.id().to_string();
                    let (it, emb, done) = with_embeddings(it, MemoryKind::KnowledgeVault, re);
                    match self.vault.insert(it, emb) {
                        Ok(_) => {
                            r.imported += 1;
                            r.embedded += usize::from(done);
                        }
                        Err(e) => r.failed.push(format!("{id}: {e}")),
                    }
                }
                Err(e) => r.failed.push(format!("vault item: {e}")),
            }
        }
        for b in &exp.core_blocks {
            match if b.user_id == exp.user { self.core.import(b) } else { Err(MemError::Serde("belongs to another user".into())) } {
                Ok(()) => r.imported += 1,
                Err(e) => r.failed.push(format!("block {}: {e}", b.label)),
            }
        }
        for e in &exp.skill_experiences {
            if e.user_id != exp.user {
                r.failed.push(format!("{}: belongs to another user", e.id));
                continue;
            }
            match self.experience.create(e.clone()) {
                Ok(_) => r.imported += 1,
                Err(err) => r.failed.push(format!("{}: {err}", e.id)),
            }
        }
        for src in &exp.sources {
            if src.user_id != exp.user {
                r.failed.push(format!("{}: belongs to another user", src.id));
                continue;
            }
            match self.sources.create(src.clone()) {
                Ok(_) => r.imported += 1,
                Err(err) => r.failed.push(format!("{}: {err}", src.id)),
            }
        }
        Ok(r)
    }

    fn agent_ids_with_experiences(&self, user: &str) -> Result<Vec<String>> {
        self.experience.agent_ids(user)
    }

    /// Retention: delete raw memories not updated for the TTL (14 days) and physically remove them from the vector-store
    /// files (a plain delete leaves the text in page files and the WAL). The audit ledger is never touched.
    pub fn purge_expired_raw(&self, now_ms: crate::types::Ms) -> Result<usize> {
        let n = crate::raw::purge_expired(&self.raw, now_ms)?;
        if n > 0 {
            self.vectors.compact(MemoryKind::Raw.collection())?;
        }
        Ok(n)
    }

    /// Erase everything about `user` and destroy their encryption key. Irreversible.
    pub fn erase_user(&self, user: &str) -> Result<ErasureReport> {
        let report = ErasureReport {
            user: user.to_string(),
            episodic: self.episodic.delete_by_user(user)?,
            semantic: self.semantic.delete_by_user(user)?,
            procedural: self.procedural.delete_by_user(user)?,
            resource: self.resource.delete_by_user(user)?,
            knowledge_vault: self.vault.delete_by_user(user)?,
            raw: self.raw.delete_by_user(user)?,
            core_blocks: self.core.delete_user(user)?,
            skill_experiences: self.experience.delete_user(user)?,
            sources: self.sources.delete_user(user)?,
            key_destroyed: false,
        };
        for k in MemoryKind::ALL {
            self.vectors.compact(k.collection())?;
        }
        let key_destroyed = self.keyring.shred(user)?;
        Ok(ErasureReport { key_destroyed, ..report })
    }
}
