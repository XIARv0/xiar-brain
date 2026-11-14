// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Own OWL 2 model for the XIAR OWL profile, with a functional-syntax reader/writer, an RDF/XML exporter and a reasoner.
//! Clean-room: designed from the W3C OWL 2 Structural Specification.

pub mod chains;
pub mod data;
pub mod functional;
pub mod model;
pub mod rdfxml;
pub mod reasoner;

#[cfg(test)]
mod reasoner_tests;
