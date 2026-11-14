// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! `MemoryRecord` implementations for each vector-backed layer.

use crate::filter_tags::Tags;
use crate::manager::{MemError, MemoryRecord, Result, Sealer};
use crate::types::*;
use base64::Engine;
use serde_json::{Map, Value};

macro_rules! common_record {
    ($t:ty, $kind:expr, occurred = $occ:expr) => {
        impl MemoryRecord for $t {
            const KIND: MemoryKind = $kind;
            fn id(&self) -> &str {
                &self.common.id
            }
            fn user_id(&self) -> &str {
                &self.common.user_id
            }
            fn client_id(&self) -> Option<&str> {
                self.common.client_id.as_deref()
            }
            fn filter_tags(&self) -> Option<&Tags> {
                self.common.filter_tags.as_ref()
            }
            fn modified_ms(&self) -> Ms {
                self.common.last_modify.timestamp_ms
            }
            fn listing_ms(&self) -> Ms {
                let f: fn(&Self) -> Option<Ms> = $occ;
                f(self).unwrap_or(self.common.created_at_ms)
            }
            fn occurred_ms(&self) -> Option<Ms> {
                let f: fn(&Self) -> Option<Ms> = $occ;
                f(self)
            }
            fn validate(&self) -> std::result::Result<(), ValidationError> {
                <$t>::validate(self)
            }
            fn set_embedding(&mut self, meta: Option<EmbeddingMeta>) {
                self.embedding = meta;
            }
        }
    };
}

common_record!(EpisodicEvent, MemoryKind::Episodic, occurred = |s| Some(s.occurred_at_ms));
common_record!(SemanticItem, MemoryKind::Semantic, occurred = |_| None);
common_record!(ProceduralItem, MemoryKind::Procedural, occurred = |_| None);
common_record!(ResourceItem, MemoryKind::Resource, occurred = |_| None);

impl MemoryRecord for RawMemory {
    const KIND: MemoryKind = MemoryKind::Raw;
    fn id(&self) -> &str {
        &self.id
    }
    fn user_id(&self) -> &str {
        &self.user_id
    }
    fn client_id(&self) -> Option<&str> {
        None
    }
    fn filter_tags(&self) -> Option<&Tags> {
        self.filter_tags.as_ref()
    }
    fn modified_ms(&self) -> Ms {
        self.last_modify.timestamp_ms
    }
    fn listing_ms(&self) -> Ms {
        self.created_at_ms
    }
    fn occurred_ms(&self) -> Option<Ms> {
        Some(self.occurred_at_ms)
    }
    fn validate(&self) -> std::result::Result<(), ValidationError> {
        RawMemory::validate(self)
    }
    fn set_embedding(&mut self, meta: Option<EmbeddingMeta>) {
        self.embedding = meta;
    }
}

fn vault_aad(id: &str) -> Vec<u8> {
    [b"xiar-vault-secret:".as_slice(), id.as_bytes()].concat()
}

/// The vault's `secret_value` is sealed with the owner's key before it reaches the vector store. The payload holds
/// only `secret_sealed`; the plaintext is never indexed, embedded or written to disk.
impl MemoryRecord for VaultItem {
    const KIND: MemoryKind = MemoryKind::KnowledgeVault;
    fn id(&self) -> &str {
        &self.common.id
    }
    fn user_id(&self) -> &str {
        &self.common.user_id
    }
    fn client_id(&self) -> Option<&str> {
        self.common.client_id.as_deref()
    }
    fn filter_tags(&self) -> Option<&Tags> {
        self.common.filter_tags.as_ref()
    }
    fn modified_ms(&self) -> Ms {
        self.common.last_modify.timestamp_ms
    }
    fn listing_ms(&self) -> Ms {
        self.common.created_at_ms
    }
    fn validate(&self) -> std::result::Result<(), ValidationError> {
        VaultItem::validate(self)
    }
    fn set_embedding(&mut self, meta: Option<EmbeddingMeta>) {
        self.embedding = meta;
    }

    fn to_payload(&self, sealer: &dyn Sealer) -> Result<Map<String, Value>> {
        let Value::Object(mut m) = serde_json::to_value(self).map_err(|e| MemError::Serde(e.to_string()))? else {
            return Err(MemError::Serde("vault item did not serialize to an object".into()));
        };
        let secret = m.remove("secret_value").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        let sealed = sealer.seal(&self.common.user_id, &vault_aad(&self.common.id), secret.as_bytes())?;
        m.insert("secret_sealed".into(), Value::String(base64::engine::general_purpose::STANDARD.encode(sealed)));
        Ok(m)
    }

    fn from_payload(mut payload: Map<String, Value>, sealer: &dyn Sealer) -> Result<Self> {
        payload.remove("_id");
        let sealed_b64 = payload.remove("secret_sealed").and_then(|v| v.as_str().map(str::to_string)).ok_or_else(|| MemError::Serde("vault payload without secret_sealed".into()))?;
        let sealed = base64::engine::general_purpose::STANDARD.decode(sealed_b64).map_err(|e| MemError::Serde(e.to_string()))?;
        let user = payload.get("user_id").and_then(Value::as_str).unwrap_or_default().to_string();
        let id = payload.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let secret = sealer.open(&user, &vault_aad(&id), &sealed)?;
        payload.insert("secret_value".into(), Value::String(String::from_utf8(secret).map_err(|e| MemError::Serde(e.to_string()))?));
        serde_json::from_value(Value::Object(payload)).map_err(|e| MemError::Serde(e.to_string()))
    }
}
