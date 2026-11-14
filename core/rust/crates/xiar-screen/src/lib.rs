// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Screen and voice monitor core. The platform parts (taking a screenshot, recording audio, transcribing, a vision
//! model) are plug-ins; this crate is the part that decides what may be looked at and what may be kept.
//!
//! Pixels never reach storage: a frame is filtered, described to text by a vision engine, scrubbed of secrets, and the
//! frame is dropped. Only the scrubbed text goes on to the memory pipeline.

pub mod privacy;
pub mod pipeline;
pub mod runner;
pub mod urls;
