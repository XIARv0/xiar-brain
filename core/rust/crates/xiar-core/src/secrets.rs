// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Secret storage. API keys and the store master key live ONLY in the OS credential store (Windows Credential
//! Manager, macOS Keychain, Secret Service on Linux). They are never written to config files, prompts, the database
//! or frontend code.

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret {0:?} not found")]
    NotFound(String),
    #[error("credential store error: {0}")]
    Backend(String),
    #[error("invalid secret name {0:?}")]
    BadName(String),
}

pub trait SecretStore: Send + Sync {
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError>;
    fn get(&self, name: &str) -> Result<String, SecretError>;
    fn delete(&self, name: &str) -> Result<(), SecretError>;

    /// Get the 32-byte master key, creating and storing a random one on first use.
    fn master_key(&self) -> Result<[u8; 32], SecretError> {
        const NAME: &str = "master-key";
        match self.get(NAME) {
            Ok(hex) => decode_key(&hex),
            Err(SecretError::NotFound(_)) => {
                let key = random_key()?;
                self.set(NAME, &encode_key(&key))?;
                Ok(key)
            }
            Err(e) => Err(e),
        }
    }
}

fn random_key() -> Result<[u8; 32], SecretError> {
    use rand::RngCore;
    let mut k = [0u8; 32];
    rand::rngs::OsRng.try_fill_bytes(&mut k).map_err(|e| SecretError::Backend(format!("OS random source failed: {e}")))?;
    Ok(k)
}

fn encode_key(k: &[u8; 32]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_key(hex: &str) -> Result<[u8; 32], SecretError> {
    if hex.len() != 64 {
        return Err(SecretError::Backend("stored master key has wrong length".into()));
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| SecretError::Backend("stored master key is not hex".into()))?;
    }
    Ok(out)
}

fn check_name(name: &str) -> Result<(), SecretError> {
    let ok = !name.is_empty() && name.len() <= 128 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b':');
    if ok { Ok(()) } else { Err(SecretError::BadName(name.to_string())) }
}

/// Real OS credential store.
pub struct OsSecretStore {
    service: String,
}

impl OsSecretStore {
    pub fn new(service: &str) -> Self {
        Self { service: service.to_string() }
    }
    fn entry(&self, name: &str) -> Result<keyring::Entry, SecretError> {
        check_name(name)?;
        keyring::Entry::new(&self.service, name).map_err(|e| SecretError::Backend(e.to_string()))
    }
}

impl SecretStore for OsSecretStore {
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.entry(name)?.set_password(value).map_err(|e| SecretError::Backend(e.to_string()))
    }
    fn get(&self, name: &str) -> Result<String, SecretError> {
        match self.entry(name)?.get_password() {
            Ok(v) => Ok(v),
            Err(keyring::Error::NoEntry) => Err(SecretError::NotFound(name.to_string())),
            Err(e) => Err(SecretError::Backend(e.to_string())),
        }
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Backend(e.to_string())),
        }
    }
}

/// In-memory store for tests and for headless CI where no credential store exists. Never use in production builds.
#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        check_name(name)?;
        self.0.lock().unwrap().insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn get(&self, name: &str) -> Result<String, SecretError> {
        check_name(name)?;
        self.0.lock().unwrap().get(name).cloned().ok_or_else(|| SecretError::NotFound(name.to_string()))
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        check_name(name)?;
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip_and_name_validation() {
        let s = MemorySecretStore::default();
        assert!(matches!(s.get("openai-key"), Err(SecretError::NotFound(_))));
        s.set("openai-key", "sk-test").unwrap();
        assert_eq!(s.get("openai-key").unwrap(), "sk-test");
        s.delete("openai-key").unwrap();
        s.delete("openai-key").unwrap(); // idempotent
        assert!(matches!(s.get("openai-key"), Err(SecretError::NotFound(_))));
        for bad in ["", "has space", "../x", "a/b"] {
            assert!(matches!(s.set(bad, "v"), Err(SecretError::BadName(_))), "{bad}");
        }
    }

    #[test]
    fn master_key_is_created_once_and_then_stable() {
        let s = MemorySecretStore::default();
        let a = s.master_key().unwrap();
        let b = s.master_key().unwrap();
        assert_eq!(a, b, "second call must return the stored key");
        assert_ne!(a, [0u8; 32]);
        let other = MemorySecretStore::default().master_key().unwrap();
        assert_ne!(a, other, "independent stores get independent random keys");
    }

    #[test]
    fn os_credential_store_roundtrip_when_available() {
        let s = OsSecretStore::new("xiar-v0-test-do-not-keep");
        if s.set("probe", "1").is_err() {
            eprintln!("no OS credential store available here; skipping");
            return;
        }
        assert_eq!(s.get("probe").unwrap(), "1");
        let k = s.master_key().unwrap();
        assert_eq!(s.master_key().unwrap(), k);
        s.delete("probe").unwrap();
        s.delete("master-key").unwrap();
        assert!(matches!(s.get("probe"), Err(SecretError::NotFound(_))));
    }

    #[test]
    fn key_hex_codec_roundtrips() {
        let k = [0xabu8; 32];
        assert_eq!(decode_key(&encode_key(&k)).unwrap(), k);
        assert!(decode_key("zz").is_err());
        assert!(decode_key(&"g".repeat(64)).is_err());
    }
}
