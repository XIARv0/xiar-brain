// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Logical checking of an ontology with the own OWL reasoner (the legacy service used Pellet through owlready2).
//! An inconsistent ontology or an unsatisfiable class is an error; a property without domain or range is a warning.

use crate::engineer::parts;
use crate::models::ConsistencyResult;
use std::time::Instant;
use xiar_owl::model::Ontology;
use xiar_owl::reasoner::{ReasonError, Reasoner};

pub const REASONER_NAME: &str = "xiar-owl (SHIQ tableau)";

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub fn check(o: &Ontology) -> ConsistencyResult {
    let start = Instant::now();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let r = Reasoner::new(o);
    match r.is_consistent() {
        Ok(false) => errors.push("The ontology is inconsistent (it has no model)".to_string()),
        Ok(true) => match r.classify() {
            Ok(k) => errors.extend(k.unsatisfiable.iter().map(|c| format!("Unsatisfiable class: {}", c.local()))),
            Err(ReasonError::Budget(w)) => errors.push(format!("Reasoning exceeded its budget ({w}); the ontology is too complex to check")),
            Err(e) => errors.push(e.to_string()),
        },
        Err(ReasonError::Budget(w)) => errors.push(format!("Reasoning exceeded its budget ({w}); the ontology is too complex to check")),
        Err(e) => errors.push(e.to_string()),
    }
    let (_, triples) = parts(o);
    for (d, p, r) in &triples {
        if d.is_empty() {
            warnings.push(format!("Property {p} has no domain specified"));
        }
        if r.is_empty() {
            warnings.push(format!("Property {p} has no range specified"));
        }
    }
    ConsistencyResult { is_consistent: errors.is_empty(), checked_at_ms: now_ms(), reasoner: REASONER_NAME.into(), errors, warnings, execution_time_ms: start.elapsed().as_secs_f64() * 1000.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiar_owl::functional::parse;

    fn check_text(axioms: &str) -> ConsistencyResult {
        check(&parse(&format!("Prefix(x:=<http://xiar.ai/ontology#>)\nOntology(<http://xiar.ai/ontology>\n{axioms}\n)")).unwrap())
    }

    #[test]
    fn a_sound_ontology_passes_and_property_gaps_are_only_warnings() {
        let r = check_text("Declaration(Class(x:A)) Declaration(ObjectProperty(x:p)) SubClassOf(x:B x:A)");
        assert!(r.is_consistent, "{:?}", r.errors);
        assert_eq!(r.warnings, ["Property p has no domain specified", "Property p has no range specified"]);
        assert_eq!(r.reasoner, REASONER_NAME);
    }

    #[test]
    fn unsatisfiable_classes_and_inconsistency_are_errors() {
        let r = check_text("DisjointClasses(x:A x:B) SubClassOf(x:C x:A) SubClassOf(x:C x:B)");
        assert!(!r.is_consistent);
        assert_eq!(r.errors, ["Unsatisfiable class: C"]);
        let r = check_text("DisjointClasses(x:A x:B) ClassAssertion(x:A x:i) ClassAssertion(x:B x:i)");
        assert_eq!(r.errors, ["The ontology is inconsistent (it has no model)"]);
    }
}
