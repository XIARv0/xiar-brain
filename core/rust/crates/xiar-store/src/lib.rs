// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! xiar-store — storage traits and pure-Rust implementations.
//!
//! * [`relational`]: `RelationalStore` trait (atomic transactions over named byte tables) + `RedbStore`.
//! * [`keyring`]: at-rest encryption with one data key per user, wrapped by a master key; deleting a user's
//!   key ("crypto-shredding") makes everything sealed for that user unrecoverable.
//!
//! The vector/search store (Qdrant Edge fork) is added behind its own trait later.

pub mod cache;
pub mod keyring;
pub mod relational;
pub mod vector;
#[cfg(test)]
mod vector_tests;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("storage error: {0}")]
    Backend(String),
    #[error("key for user {0:?} was erased (crypto-shredded); data is unrecoverable")]
    KeyErased(String),
    #[error("the data of user {0:?} is locked: it opens when that account signs in with its password")]
    Locked(String),
    #[error("decryption failed (wrong key, tampered data or mismatched context)")]
    Decrypt,
    #[error("vector {name:?}: expected dimension {expected}, got {got}")]
    DimensionMismatch { name: String, expected: usize, got: usize },
    #[error("malformed stored value: {0}")]
    Malformed(&'static str),
}

pub type Result<T> = std::result::Result<T, StoreError>;

impl StoreError {
    pub(crate) fn backend(e: impl std::fmt::Display) -> Self {
        StoreError::Backend(e.to_string())
    }
}
