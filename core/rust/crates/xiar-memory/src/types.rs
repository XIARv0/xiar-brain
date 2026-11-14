// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Memory item types. Field names and meanings follow MIRIX `schemas/*`. Timestamps are unix milliseconds (UTC).
//! Embeddings are NOT part of the item: they live as named vectors in the vector store (see [`MemoryKind::vector_fields`]);
//! items only record which embedding configuration produced them.

use crate::filter_tags::Tags;
use crate::ids;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type Ms = i64;

pub fn now_ms() -> Ms {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("{0} must not be empty")]
    Empty(&'static str),
    #[error("{field} is longer than {max} characters")]
    TooLong { field: &'static str, max: usize },
    #[error("{field} must be one of {allowed:?}, got {got:?}")]
    NotAllowed { field: &'static str, allowed: &'static [&'static str], got: String },
    #[error("{0} must be between 0 and 1")]
    OutOfRange(&'static str),
    #[error("block value is {len} characters, over the {limit} character limit")]
    BlockTooLong { len: usize, limit: usize },
}

pub const CORE_MEMORY_BLOCK_CHAR_LIMIT: usize = 5000;
pub const PROCEDURAL_ENTRY_TYPES: &[&str] = &["workflow", "guide", "script"];
pub const SENSITIVITY_LEVELS: &[&str] = &["low", "medium", "high"];

fn non_empty(name: &'static str, v: &str) -> Result<(), ValidationError> {
    if v.trim().is_empty() { Err(ValidationError::Empty(name)) } else { Ok(()) }
}

/// Which embedding configuration produced an item's vectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingMeta {
    pub plugin: String,
    pub model: String,
    pub dim: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastModify {
    pub timestamp_ms: Ms,
    /// "created", "updated", or a comma-joined list of operations (MIRIX convention)
    pub operation: String,
}

impl LastModify {
    pub fn now(operation: &str) -> Self {
        Self { timestamp_ms: now_ms(), operation: operation.to_string() }
    }
}

/// Fields every memory item shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Common {
    pub id: String,
    pub agent_id: Option<String>,
    /// the client application that created the item (a local app/bot id; kept for scope handling)
    pub client_id: Option<String>,
    pub user_id: String,
    pub created_at_ms: Ms,
    pub updated_at_ms: Option<Ms>,
    pub last_modify: LastModify,
    pub filter_tags: Option<Tags>,
    /// ids of the source records (screenshot, conversation, document...) this item was made from; see `sources`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<String>,
}

impl Common {
    pub fn new(prefix: &str, user_id: &str) -> Self {
        let now = now_ms();
        Self {
            id: ids::new_id(prefix),
            agent_id: None,
            client_id: None,
            user_id: user_id.to_string(),
            created_at_ms: now,
            updated_at_ms: None,
            last_modify: LastModify { timestamp_ms: now, operation: "created".into() },
            filter_tags: None,
            source_refs: Vec::new(),
        }
    }

    fn touch(&mut self, operation: &str) {
        let now = now_ms();
        self.updated_at_ms = Some(now);
        self.last_modify = LastModify { timestamp_ms: now, operation: operation.to_string() };
    }
}

/// The searchable memory layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryKind {
    Episodic,
    Semantic,
    Procedural,
    Resource,
    KnowledgeVault,
    Raw,
}

impl MemoryKind {
    pub const ALL: [MemoryKind; 6] = [Self::Episodic, Self::Semantic, Self::Procedural, Self::Resource, Self::KnowledgeVault, Self::Raw];

    /// Vector-store collection name.
    pub fn collection(self) -> &'static str {
        match self {
            Self::Episodic => "episodic",
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
            Self::Resource => "resource",
            Self::KnowledgeVault => "knowledge_vault",
            Self::Raw => "raw",
        }
    }

    pub fn id_prefix(self) -> &'static str {
        match self {
            Self::Episodic => ids::EPISODIC,
            Self::Semantic => ids::SEMANTIC,
            Self::Procedural => ids::PROCEDURAL,
            Self::Resource => ids::RESOURCE,
            Self::KnowledgeVault => ids::KNOWLEDGE_VAULT,
            Self::Raw => ids::RAW,
        }
    }

    /// Named dense vectors per layer (the fields MIRIX embeds).
    pub fn vector_fields(self) -> &'static [&'static str] {
        match self {
            Self::Episodic => &["summary", "details"],
            Self::Semantic => &["name", "summary", "details"],
            Self::Procedural => &["description", "instructions"],
            Self::Resource => &["summary"],
            Self::KnowledgeVault => &["caption"],
            Self::Raw => &["context"],
        }
    }

    /// Text fields that feed BM25 / string matching. The vault's `secret_value` is deliberately NOT searchable.
    pub fn text_fields(self) -> &'static [&'static str] {
        match self {
            Self::Episodic => &["event_type", "summary", "details", "actor"],
            Self::Semantic => &["name", "summary", "details", "source", "entity_types"],
            Self::Procedural => &["name", "entry_type", "description", "instructions"],
            Self::Resource => &["title", "summary", "resource_type", "content"],
            Self::KnowledgeVault => &["entry_type", "source", "caption"],
            Self::Raw => &["context"],
        }
    }
}

// ------------------------------------------------------------------ episodic
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodicEvent {
    #[serde(flatten)]
    pub common: Common,
    /// e.g. user_message, inference, system_notification
    pub event_type: String,
    pub summary: String,
    pub details: String,
    /// "user" or "assistant"
    pub actor: String,
    pub occurred_at_ms: Ms,
    pub embedding: Option<EmbeddingMeta>,
}

impl EpisodicEvent {
    pub fn new(user_id: &str, event_type: &str, summary: &str, details: &str, actor: &str, occurred_at_ms: Ms) -> Self {
        Self {
            common: Common::new(ids::EPISODIC, user_id),
            event_type: event_type.into(),
            summary: summary.into(),
            details: details.into(),
            actor: actor.into(),
            occurred_at_ms,
            embedding: None,
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        non_empty("event_type", &self.event_type)?;
        non_empty("summary", &self.summary)?;
        non_empty("actor", &self.actor)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EpisodicUpdate {
    pub event_type: Option<String>,
    pub summary: Option<String>,
    pub details: Option<String>,
    pub occurred_at_ms: Option<Ms>,
    pub filter_tags: Option<Tags>,
}

impl EpisodicUpdate {
    /// Apply to an item; returns the names of the fields that changed (used for `last_modify.operation`).
    pub fn apply(&self, e: &mut EpisodicEvent) -> Vec<&'static str> {
        let mut changed = vec![];
        macro_rules! set { ($f:ident) => { if let Some(v) = &self.$f { if *v != e.$f { e.$f = v.clone(); changed.push(stringify!($f)); } } }; }
        set!(event_type);
        set!(summary);
        set!(details);
        if let Some(v) = self.occurred_at_ms { if v != e.occurred_at_ms { e.occurred_at_ms = v; changed.push("occurred_at"); } }
        if let Some(v) = &self.filter_tags { if Some(v) != e.common.filter_tags.as_ref() { e.common.filter_tags = Some(v.clone()); changed.push("filter_tags"); } }
        if !changed.is_empty() { e.common.touch(&changed.join(", ")); }
        changed
    }
}

// ------------------------------------------------------------------ semantic
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticItem {
    #[serde(flatten)]
    pub common: Common,
    pub name: String,
    pub summary: String,
    pub details: String,
    /// where the knowledge came from (book, article, conversation...)
    pub source: String,
    /// ontology class names the entity was asserted to belong to (validated against the promoted ontology when written);
    /// what follows from them (superclasses) is worked out when reading, never stored
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_types: Vec<String>,
    pub embedding: Option<EmbeddingMeta>,
}

impl SemanticItem {
    pub fn new(user_id: &str, name: &str, summary: &str, details: &str, source: &str) -> Self {
        Self { common: Common::new(ids::SEMANTIC, user_id), name: name.into(), summary: summary.into(), details: details.into(), source: source.into(), entity_types: Vec::new(), embedding: None }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        non_empty("name", &self.name)?;
        non_empty("summary", &self.summary)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SemanticUpdate {
    pub name: Option<String>,
    pub summary: Option<String>,
    pub details: Option<String>,
    pub source: Option<String>,
    pub entity_types: Option<Vec<String>>,
    pub filter_tags: Option<Tags>,
}

impl SemanticUpdate {
    pub fn apply(&self, e: &mut SemanticItem) -> Vec<&'static str> {
        let mut changed = vec![];
        macro_rules! set { ($f:ident) => { if let Some(v) = &self.$f { if *v != e.$f { e.$f = v.clone(); changed.push(stringify!($f)); } } }; }
        set!(name);
        set!(summary);
        set!(details);
        set!(source);
        set!(entity_types);
        if let Some(v) = &self.filter_tags { if Some(v) != e.common.filter_tags.as_ref() { e.common.filter_tags = Some(v.clone()); changed.push("filter_tags"); } }
        if !changed.is_empty() { e.common.touch(&changed.join(", ")); }
        changed
    }
}

// ------------------------------------------------------------------ procedural (skills)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralItem {
    #[serde(flatten)]
    pub common: Common,
    /// short skill identifier, unique per user (e.g. "deploy-production")
    pub name: String,
    /// workflow | guide | script
    pub entry_type: String,
    pub description: String,
    pub instructions: String,
    pub triggers: Vec<String>,
    pub examples: Vec<Value>,
    /// semantic version of the skill
    pub version: String,
    pub embedding: Option<EmbeddingMeta>,
}

impl ProceduralItem {
    pub fn new(user_id: &str, name: &str, entry_type: &str, description: &str, instructions: &str) -> Self {
        Self {
            common: Common::new(ids::PROCEDURAL, user_id),
            name: name.into(),
            entry_type: entry_type.into(),
            description: description.into(),
            instructions: instructions.into(),
            triggers: vec![],
            examples: vec![],
            version: "0.1.0".into(),
            embedding: None,
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        non_empty("name", &self.name)?;
        non_empty("description", &self.description)?;
        if !PROCEDURAL_ENTRY_TYPES.contains(&self.entry_type.as_str()) {
            return Err(ValidationError::NotAllowed { field: "entry_type", allowed: PROCEDURAL_ENTRY_TYPES, got: self.entry_type.clone() });
        }
        // the pinned upstream's schema limits
        if self.triggers.iter().any(|t| t.chars().count() > 512) {
            return Err(ValidationError::TooLong { field: "triggers", max: 512 });
        }
        if self.examples.len() > 50 {
            return Err(ValidationError::TooLong { field: "examples", max: 50 });
        }
        if self.examples.iter().any(|e| !e.is_object()) {
            return Err(ValidationError::NotAllowed { field: "examples", allowed: &["object"], got: "non-object entry".into() });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProceduralUpdate {
    pub name: Option<String>,
    pub entry_type: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub triggers: Option<Vec<String>>,
    pub examples: Option<Vec<Value>>,
    pub version: Option<String>,
    pub filter_tags: Option<Tags>,
}

impl ProceduralUpdate {
    pub fn apply(&self, e: &mut ProceduralItem) -> Vec<&'static str> {
        let mut changed = vec![];
        macro_rules! set { ($f:ident) => { if let Some(v) = &self.$f { if *v != e.$f { e.$f = v.clone(); changed.push(stringify!($f)); } } }; }
        set!(name);
        set!(entry_type);
        set!(description);
        set!(instructions);
        set!(triggers);
        set!(examples);
        set!(version);
        if let Some(v) = &self.filter_tags { if Some(v) != e.common.filter_tags.as_ref() { e.common.filter_tags = Some(v.clone()); changed.push("filter_tags"); } }
        if !changed.is_empty() { e.common.touch(&changed.join(", ")); }
        changed
    }
}

// ------------------------------------------------------------------ resource
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceItem {
    #[serde(flatten)]
    pub common: Common,
    pub title: String,
    pub summary: String,
    /// doc, markdown, pdf_text, ...
    pub resource_type: String,
    pub content: String,
    pub embedding: Option<EmbeddingMeta>,
}

impl ResourceItem {
    pub fn new(user_id: &str, title: &str, summary: &str, resource_type: &str, content: &str) -> Self {
        Self { common: Common::new(ids::RESOURCE, user_id), title: title.into(), summary: summary.into(), resource_type: resource_type.into(), content: content.into(), embedding: None }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        non_empty("title", &self.title)?;
        non_empty("resource_type", &self.resource_type)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceUpdate {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub resource_type: Option<String>,
    pub content: Option<String>,
    pub filter_tags: Option<Tags>,
}

impl ResourceUpdate {
    pub fn apply(&self, e: &mut ResourceItem) -> Vec<&'static str> {
        let mut changed = vec![];
        macro_rules! set { ($f:ident) => { if let Some(v) = &self.$f { if *v != e.$f { e.$f = v.clone(); changed.push(stringify!($f)); } } }; }
        set!(title);
        set!(summary);
        set!(resource_type);
        set!(content);
        if let Some(v) = &self.filter_tags { if Some(v) != e.common.filter_tags.as_ref() { e.common.filter_tags = Some(v.clone()); changed.push("filter_tags"); } }
        if !changed.is_empty() { e.common.touch(&changed.join(", ")); }
        changed
    }
}

// ------------------------------------------------------------------ knowledge vault
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultItem {
    #[serde(flatten)]
    pub common: Common,
    /// credential, bookmark, api_key, ...
    pub entry_type: String,
    pub source: String,
    /// low | medium | high
    pub sensitivity: String,
    /// The secret itself. ALWAYS sealed with the user's key at rest; never indexed, never embedded, never logged.
    pub secret_value: String,
    /// human description ("API key for the OpenAI service"); this is what gets embedded and searched
    pub caption: String,
    pub embedding: Option<EmbeddingMeta>,
}

// `secret_value` must never reach logs through Debug.
impl std::fmt::Debug for VaultItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultItem")
            .field("id", &self.common.id)
            .field("user_id", &self.common.user_id)
            .field("entry_type", &self.entry_type)
            .field("sensitivity", &self.sensitivity)
            .field("caption", &self.caption)
            .field("secret_value", &"<redacted>")
            .finish()
    }
}

impl VaultItem {
    pub fn new(user_id: &str, entry_type: &str, source: &str, sensitivity: &str, secret_value: &str, caption: &str) -> Self {
        Self { common: Common::new(ids::KNOWLEDGE_VAULT, user_id), entry_type: entry_type.into(), source: source.into(), sensitivity: sensitivity.into(), secret_value: secret_value.into(), caption: caption.into(), embedding: None }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        non_empty("entry_type", &self.entry_type)?;
        non_empty("caption", &self.caption)?;
        if !SENSITIVITY_LEVELS.contains(&self.sensitivity.as_str()) {
            return Err(ValidationError::NotAllowed { field: "sensitivity", allowed: SENSITIVITY_LEVELS, got: self.sensitivity.clone() });
        }
        Ok(())
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct VaultUpdate {
    pub entry_type: Option<String>,
    pub source: Option<String>,
    pub sensitivity: Option<String>,
    pub secret_value: Option<String>,
    pub caption: Option<String>,
    pub filter_tags: Option<Tags>,
}

impl std::fmt::Debug for VaultUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultUpdate").field("secret_value", &self.secret_value.as_ref().map(|_| "<redacted>")).field("caption", &self.caption).finish()
    }
}

impl VaultUpdate {
    pub fn apply(&self, e: &mut VaultItem) -> Vec<&'static str> {
        let mut changed = vec![];
        macro_rules! set { ($f:ident) => { if let Some(v) = &self.$f { if *v != e.$f { e.$f = v.clone(); changed.push(stringify!($f)); } } }; }
        set!(entry_type);
        set!(source);
        set!(sensitivity);
        set!(secret_value);
        set!(caption);
        if let Some(v) = &self.filter_tags { if Some(v) != e.common.filter_tags.as_ref() { e.common.filter_tags = Some(v.clone()); changed.push("filter_tags"); } }
        if !changed.is_empty() { e.common.touch(&changed.join(", ")); }
        changed
    }
}

// ------------------------------------------------------------------ raw memory (14-day TTL)
pub const RAW_MEMORY_TTL_DAYS: i64 = 14;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawMemory {
    /// UUIDv7
    pub id: String,
    pub user_id: String,
    pub context: String,
    pub filter_tags: Option<Tags>,
    pub occurred_at_ms: Ms,
    pub created_at_ms: Ms,
    pub updated_at_ms: Ms,
    pub last_modify: LastModify,
    pub embedding: Option<EmbeddingMeta>,
}

impl RawMemory {
    pub fn new(user_id: &str, context: &str) -> Self {
        let now = now_ms();
        Self { id: ids::new_raw_id(), user_id: user_id.into(), context: context.into(), filter_tags: None, occurred_at_ms: now, created_at_ms: now, updated_at_ms: now, last_modify: LastModify { timestamp_ms: now, operation: "created".into() }, embedding: None }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        non_empty("context", &self.context)
    }
    /// Expired when last updated more than 14 days before `now_ms` (MIRIX bases TTL on `updated_at`).
    pub fn is_expired(&self, now_ms: Ms) -> bool {
        now_ms - self.updated_at_ms > RAW_MEMORY_TTL_DAYS * 24 * 3600 * 1000
    }
}

// ------------------------------------------------------------------ skill experience
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceType {
    WorthLearning,
    WorthAvoiding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceStatus {
    Pending,
    Consumed,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillExperience {
    pub id: String,
    pub agent_id: Option<String>,
    pub user_id: String,
    /// provenance: the session this was distilled from
    pub session_id: String,
    pub experience_type: ExperienceType,
    pub title: String,
    /// the transferable lesson
    pub content: String,
    pub importance: f32,
    pub credibility: f32,
    /// JSON string `{quote, signal_type}`
    pub evidence: String,
    pub status: ExperienceStatus,
    pub consumed_by: Option<String>,
    pub influenced_skill_ids: Option<Vec<String>>,
    pub created_at_ms: Ms,
    pub updated_at_ms: Option<Ms>,
}

impl SkillExperience {
    pub fn new(user_id: &str, session_id: &str, experience_type: ExperienceType, title: &str, content: &str, importance: f32, credibility: f32, evidence: &str) -> Self {
        Self {
            id: ids::new_id(ids::SKILL_EXPERIENCE),
            agent_id: None,
            user_id: user_id.into(),
            session_id: session_id.into(),
            experience_type,
            title: title.into(),
            content: content.into(),
            importance,
            credibility,
            evidence: evidence.into(),
            status: ExperienceStatus::Pending,
            consumed_by: None,
            influenced_skill_ids: None,
            created_at_ms: now_ms(),
            updated_at_ms: None,
        }
    }
    /// Pull both scores into `[0, 1]` the way the pinned upstream's schema does: out-of-range values clamp, NaN becomes 0.
    pub fn clamp_scores(&mut self) {
        let clamp = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        self.importance = clamp(self.importance);
        self.credibility = clamp(self.credibility);
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        non_empty("title", &self.title)?;
        non_empty("session_id", &self.session_id)?;
        for (field, v, max) in [("title", &self.title, 256), ("content", &self.content, 8192), ("evidence", &self.evidence, 2048)] {
            if v.chars().count() > max {
                return Err(ValidationError::TooLong { field, max });
            }
        }
        if !(0.0..=1.0).contains(&self.importance) {
            return Err(ValidationError::OutOfRange("importance"));
        }
        if !(0.0..=1.0).contains(&self.credibility) {
            return Err(ValidationError::OutOfRange("credibility"));
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ core memory blocks
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    pub user_id: String,
    /// "human", "persona", or a custom label
    pub label: String,
    pub value: String,
    /// character limit; the agent is told to rewrite at 90% (see `usage_percent`)
    pub limit: usize,
    pub filter_tags: Option<Tags>,
    pub created_by_id: Option<String>,
    pub last_updated_by_id: Option<String>,
}

impl Block {
    pub fn new(user_id: &str, label: &str, value: &str) -> Self {
        Self { id: ids::new_id(ids::BLOCK), user_id: user_id.into(), label: label.into(), value: value.into(), limit: CORE_MEMORY_BLOCK_CHAR_LIMIT, filter_tags: None, created_by_id: None, last_updated_by_id: None }
    }
    pub fn len_chars(&self) -> usize {
        self.value.chars().count()
    }
    pub fn usage_percent(&self) -> f32 {
        if self.limit == 0 { 100.0 } else { self.len_chars() as f32 * 100.0 / self.limit as f32 }
    }
    /// MIRIX warns the agent ("NEARLY FULL") and asks for a rewrite at 90% usage.
    pub fn nearly_full(&self) -> bool {
        self.usage_percent() >= 90.0
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        non_empty("label", &self.label)?;
        if self.len_chars() > self.limit {
            return Err(ValidationError::BlockTooLong { len: self.len_chars(), limit: self.limit });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_items_have_prefixed_ids_and_created_metadata() {
        let e = EpisodicEvent::new("u1", "user_message", "s", "d", "user", 1_000);
        assert!(ids::has_prefix(&e.common.id, ids::EPISODIC));
        assert_eq!(e.common.last_modify.operation, "created");
        assert!(e.common.updated_at_ms.is_none());
        assert!(ids::has_prefix(&SemanticItem::new("u", "n", "s", "d", "src").common.id, ids::SEMANTIC));
        assert!(ids::has_prefix(&ProceduralItem::new("u", "n", "workflow", "d", "i").common.id, ids::PROCEDURAL));
        assert!(ids::has_prefix(&ResourceItem::new("u", "t", "s", "doc", "c").common.id, ids::RESOURCE));
        assert!(ids::has_prefix(&VaultItem::new("u", "api_key", "user", "high", "sk-1", "OpenAI key").common.id, ids::KNOWLEDGE_VAULT));
    }

    #[test]
    fn update_records_only_changed_fields_and_touches_metadata() {
        let mut e = EpisodicEvent::new("u1", "user_message", "old", "d", "user", 1_000);
        let changed = EpisodicUpdate { summary: Some("new".into()), details: Some("d".into()), ..Default::default() }.apply(&mut e);
        assert_eq!(changed, vec!["summary"], "unchanged details are not reported");
        assert_eq!(e.summary, "new");
        assert_eq!(e.common.last_modify.operation, "summary");
        assert!(e.common.updated_at_ms.is_some());
        let before = e.common.last_modify.clone();
        assert!(EpisodicUpdate::default().apply(&mut e).is_empty());
        assert_eq!(e.common.last_modify, before, "no-op update leaves metadata alone");
        let changed = EpisodicUpdate { summary: Some("x".into()), occurred_at_ms: Some(5), ..Default::default() }.apply(&mut e);
        assert_eq!(e.common.last_modify.operation, "summary, occurred_at");
        assert_eq!(changed.len(), 2);
    }

    #[test]
    fn validation_rules() {
        assert_eq!(EpisodicEvent::new("u", "", "s", "d", "user", 0).validate(), Err(ValidationError::Empty("event_type")));
        assert!(ProceduralItem::new("u", "deploy", "workflow", "d", "i").validate().is_ok());
        assert!(matches!(ProceduralItem::new("u", "deploy", "recipe", "d", "i").validate(), Err(ValidationError::NotAllowed { field: "entry_type", .. })));
        assert!(matches!(VaultItem::new("u", "api_key", "s", "top-secret", "v", "c").validate(), Err(ValidationError::NotAllowed { field: "sensitivity", .. })));
        let mut p = ProceduralItem::new("u", "deploy", "workflow", "d", "i");
        p.triggers = vec!["x".repeat(513)];
        assert_eq!(p.validate(), Err(ValidationError::TooLong { field: "triggers", max: 512 }));
        p.triggers.clear();
        p.examples = vec![serde_json::json!({}); 51];
        assert_eq!(p.validate(), Err(ValidationError::TooLong { field: "examples", max: 50 }));
        let mut x = SkillExperience::new("u", "s1", ExperienceType::WorthLearning, "t", "c", 0.5, 0.5, "{}");
        assert!(x.validate().is_ok());
        x.importance = 1.5;
        assert_eq!(x.validate(), Err(ValidationError::OutOfRange("importance")));
        x.clamp_scores();
        assert!(x.validate().is_ok() && x.importance == 1.0);
    }

    #[test]
    fn vault_secret_never_appears_in_debug_output() {
        let v = VaultItem::new("u", "api_key", "user", "high", "sk-SUPERSECRET", "OpenAI key");
        let dbg = format!("{v:?} {:?}", VaultUpdate { secret_value: Some("sk-NEW".into()), ..Default::default() });
        assert!(!dbg.contains("SUPERSECRET") && !dbg.contains("sk-NEW"), "{dbg}");
        assert!(dbg.contains("<redacted>"));
        assert!(!MemoryKind::KnowledgeVault.text_fields().contains(&"secret_value"), "secret must not be searchable");
    }

    #[test]
    fn block_usage_and_limits() {
        let mut b = Block::new("u", "human", &"x".repeat(4500));
        assert!(b.nearly_full());
        assert!((b.usage_percent() - 90.0).abs() < 0.01);
        assert!(b.validate().is_ok());
        b.value = "x".repeat(5001);
        assert_eq!(b.validate(), Err(ValidationError::BlockTooLong { len: 5001, limit: 5000 }));
        assert!(!Block::new("u", "persona", "short").nearly_full());
        assert_eq!(Block::new("u", "human", "héllo").len_chars(), 5, "limit counts characters, not bytes");
    }

    #[test]
    fn raw_memory_expires_after_14_days_of_inactivity() {
        let mut r = RawMemory::new("u", "ctx");
        let day = 24 * 3600 * 1000;
        assert!(!r.is_expired(r.updated_at_ms + 14 * day));
        assert!(r.is_expired(r.updated_at_ms + 14 * day + 1));
        r.updated_at_ms += 10 * day; // touching it extends life
        assert!(!r.is_expired(r.created_at_ms + 20 * day));
    }

    #[test]
    fn kind_tables_cover_every_embedded_and_searchable_field() {
        for k in MemoryKind::ALL {
            assert!(!k.vector_fields().is_empty() && !k.text_fields().is_empty());
            assert!(!k.collection().is_empty() && !k.id_prefix().is_empty());
        }
        assert_eq!(MemoryKind::Semantic.vector_fields(), &["name", "summary", "details"]);
    }

    #[test]
    fn items_serialize_flat_and_roundtrip() {
        let mut e = EpisodicEvent::new("u1", "user_message", "s", "d", "user", 42);
        e.common.filter_tags = json!({"scope": "work"}).as_object().cloned();
        let j = serde_json::to_value(&e).unwrap();
        assert_eq!(j["user_id"], "u1");
        assert_eq!(j["event_type"], "user_message");
        assert_eq!(j["filter_tags"]["scope"], "work");
        let back: EpisodicEvent = serde_json::from_value(j).unwrap();
        assert_eq!(back, e);
        assert_eq!(serde_json::to_value(ExperienceType::WorthAvoiding).unwrap(), "worth_avoiding");
        assert_eq!(serde_json::to_value(ExperienceStatus::Superseded).unwrap(), "superseded");
    }
}
