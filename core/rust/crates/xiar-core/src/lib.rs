// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! xiar-core — host-level building blocks.
//!
//! * [`manifest`]: the privacy manifest every plug-in declares, and the generated privacy policy.
//! * [`secrets`]: OS-keychain wrapper (API keys and the master key never touch files, config or the database).
//!
//! * [`config`]: typed TOML config with routing + privacy invariants; secrets are rejected.
//! * [`events`]: in-process pub/sub bus with bounded queues.
//!
//! Native plug-in loading lives in the separate `xiar-plugin-host` crate (the only one allowed `unsafe`).

pub mod config;
pub mod events;
pub mod manifest;
pub mod secrets;
