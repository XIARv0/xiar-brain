// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The engineer: merges candidate classes and relations into a stable ontology. Deterministic (no model call): CamelCase
//! classes, camelCase properties, fuzzy duplicate detection (similarity 0.75 or one name inside the other), relations only
//! between classes that exist, and a readable diff summary. Port of the legacy `OntologyEngineer`.

use crate::models::*;
use crate::text::{class_name, property_name, sequence_ratio};
use std::collections::{BTreeMap, BTreeSet};
use xiar_owl::model::*;

pub const SIMILARITY_THRESHOLD: f64 = 0.75;

fn iri(local: &str) -> Iri {
    Iri::new(format!("{NS}{local}"))
}

/// The class names and `(domain, predicate, range)` triples an ontology currently holds. A property without a domain or
/// range gets an empty string there.
pub fn parts(o: &Ontology) -> (BTreeSet<String>, BTreeSet<(String, String, String)>) {
    let classes: BTreeSet<String> = o.axioms.iter().filter_map(|a| if let Axiom::DeclareClass(i) = a { Some(i.local().to_string()) } else { None }).collect();
    let mut props: BTreeMap<String, (String, String)> = BTreeMap::new();
    let named = |c: &ClassExpr| if let ClassExpr::Class(i) = c { Some(i.local().to_string()) } else { None };
    for a in &o.axioms {
        match a {
            Axiom::DeclareObjectProperty(i) => {
                props.entry(i.local().to_string()).or_default();
            }
            Axiom::ObjectPropertyDomain(ObjectProp::Named(p), c) => {
                if let Some(n) = named(c) {
                    props.entry(p.local().to_string()).or_default().0 = n;
                }
            }
            Axiom::ObjectPropertyRange(ObjectProp::Named(p), c) => {
                if let Some(n) = named(c) {
                    props.entry(p.local().to_string()).or_default().1 = n;
                }
            }
            _ => {}
        }
    }
    (classes, props.into_iter().map(|(p, (d, r))| (d, p, r)).collect())
}

/// An existing name that `candidate` duplicates, if any.
pub fn find_duplicate<'a>(candidate: &str, existing: impl IntoIterator<Item = &'a String>) -> Option<&'a String> {
    let c = candidate.to_lowercase();
    existing.into_iter().find(|e| {
        let e = e.to_lowercase();
        c == e || sequence_ratio(&c, &e) >= SIMILARITY_THRESHOLD || c.contains(&e) || e.contains(&c)
    })
}

/// v1 -> v2; anything that does not start with `v<number>` becomes v2.
pub fn increment_version(current: &str) -> String {
    let digits: String = current.strip_prefix('v').unwrap_or("").chars().take_while(char::is_ascii_digit).collect();
    match digits.parse::<u64>() {
        Ok(n) if current.starts_with('v') => format!("v{}", n + 1),
        _ => "v2".to_string(),
    }
}

/// Build an ontology from class names and relation triples, in the legacy layout: classes sorted, one object property per
/// predicate with the alphabetically first domain and range.
pub fn build(classes: &BTreeSet<String>, relations: &BTreeSet<(String, String, String)>) -> Ontology {
    let mut o = Ontology { iri: Some(Iri::new(ONTOLOGY_IRI)), prefixes: vec![("xiar".into(), NS.into())], ..Ontology::default() };
    for c in classes {
        o.axioms.push(Axiom::DeclareClass(iri(c)));
    }
    let mut info: BTreeMap<&str, (BTreeSet<&str>, BTreeSet<&str>)> = BTreeMap::new();
    for (d, p, r) in relations {
        if p.is_empty() {
            continue;
        }
        let e = info.entry(p).or_default();
        if !d.is_empty() {
            e.0.insert(d);
        }
        if !r.is_empty() {
            e.1.insert(r);
        }
    }
    for (p, (domains, ranges)) in info {
        o.axioms.push(Axiom::DeclareObjectProperty(iri(p)));
        if let Some(d) = domains.first() {
            o.axioms.push(Axiom::ObjectPropertyDomain(ObjectProp::Named(iri(p)), ClassExpr::Class(iri(d))));
        }
        if let Some(r) = ranges.first() {
            o.axioms.push(Axiom::ObjectPropertyRange(ObjectProp::Named(iri(p)), ClassExpr::Class(iri(r))));
        }
    }
    o
}

/// Merge `candidates` into `base` (version `current_version`). Returns the new ontology and what changed. Subclass
/// axioms and anything else in `base` beyond classes and simple relations are carried over unchanged.
pub fn engineer(candidates: &OntologyCandidate, base: &Ontology, current_version: &str) -> (Ontology, EngineerResponse) {
    let (old_classes, old_relations) = parts(base);
    let mut classes = old_classes.clone();
    let (mut added, mut conflicts) = (0, 0);
    for c in &candidates.classes {
        let name = class_name(&c.name);
        if name.is_empty() {
            continue;
        }
        if find_duplicate(&name, &classes).is_some() {
            conflicts += 1;
        } else {
            classes.insert(name);
            added += 1;
        }
    }
    let mut relations = old_relations.clone();
    let mut relations_added = 0;
    for r in &candidates.relations {
        let (s, p, o) = (class_name(&r.subject_class), property_name(&r.predicate), class_name(&r.object_class));
        if !classes.contains(&s) || !classes.contains(&o) {
            continue;
        }
        if relations.insert((s, p, o)) {
            relations_added += 1;
        }
    }
    let mut onto = build(&classes, &relations);
    // keep axioms the generator layout does not produce (subclass hierarchies, characteristics, individuals ...)
    for a in &base.axioms {
        let generated = matches!(a, Axiom::DeclareClass(_) | Axiom::DeclareObjectProperty(_) | Axiom::ObjectPropertyDomain(ObjectProp::Named(_), ClassExpr::Class(_)) | Axiom::ObjectPropertyRange(ObjectProp::Named(_), ClassExpr::Class(_)));
        if !generated && !onto.axioms.contains(a) {
            onto.axioms.push(a.clone());
        }
    }
    let new_classes: Vec<&String> = classes.difference(&old_classes).collect();
    let mut summary = format!("Ontology Engineering Summary:\n  Classes added: {added}\n  Relations added: {relations_added}\n  Conflicts resolved: {conflicts}");
    if !new_classes.is_empty() {
        summary.push_str(&format!("\n  New classes: {}", new_classes.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    }
    let response = EngineerResponse { new_version: increment_version(current_version), diff_summary: summary, classes_added: added, relations_added, conflicts_resolved: conflicts };
    (onto, response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str) -> CandidateClass {
        CandidateClass { name: name.into(), description: String::new(), source_document: "d".into(), confidence: 0.9, occurrences: 1 }
    }

    fn rel(s: &str, p: &str, o: &str) -> CandidateRelation {
        CandidateRelation { subject_class: s.into(), predicate: p.into(), object_class: o.into(), description: String::new(), source_document: "d".into(), confidence: 0.8, occurrences: 1 }
    }

    fn base() -> Ontology {
        let classes: BTreeSet<String> = ["Agent", "Task"].iter().map(|s| s.to_string()).collect();
        let rels: BTreeSet<(String, String, String)> = [("Agent".to_string(), "uses".to_string(), "Task".to_string())].into_iter().collect();
        build(&classes, &rels)
    }

    #[test]
    fn candidates_are_normalised_deduplicated_and_summarised() {
        let cands = OntologyCandidate {
            classes: vec![class("customer order"), class("agents"), class("TASK"), class("invoice"), class("invoices"), class("work_flow"), class("")],
            relations: vec![rel("Customer Order", "is placed by", "Agent"), rel("Agent", "uses", "Task"), rel("Agent", "owns", "Ghost"), rel("invoice", "billed to", "customer order")],
            ..Default::default()
        };
        let (o, r) = engineer(&cands, &base(), "v1");
        let (classes, relations) = parts(&o);
        // "agents" ~ Agent, "TASK" == Task, "invoices" ~ Invoice; "" is ignored
        assert_eq!(classes.iter().map(String::as_str).collect::<Vec<_>>(), ["Agent", "CustomerOrder", "Invoice", "Task", "WorkFlow"]);
        assert_eq!((r.classes_added, r.conflicts_resolved), (3, 3));
        assert!(relations.contains(&("CustomerOrder".into(), "isPlacedBy".into(), "Agent".into())));
        assert!(relations.contains(&("Invoice".into(), "billedTo".into(), "CustomerOrder".into())));
        assert!(!relations.iter().any(|(_, p, _)| p == "owns"), "a relation to a missing class is dropped");
        assert_eq!(r.relations_added, 2, "uses already existed");
        assert_eq!(r.new_version, "v2");
        assert_eq!(r.diff_summary, "Ontology Engineering Summary:\n  Classes added: 3\n  Relations added: 2\n  Conflicts resolved: 3\n  New classes: CustomerOrder, Invoice, WorkFlow");
    }

    #[test]
    fn a_candidate_can_duplicate_a_class_added_earlier_in_the_same_batch() {
        let cands = OntologyCandidate { classes: vec![class("Document"), class("documents")], ..Default::default() };
        let (_, r) = engineer(&cands, &Ontology::default(), "v1");
        assert_eq!((r.classes_added, r.conflicts_resolved), (1, 1));
    }

    #[test]
    fn substrings_count_as_duplicates_unrelated_names_do_not() {
        let existing: BTreeSet<String> = ["Workflow".to_string()].into_iter().collect();
        assert!(find_duplicate("Work", &existing).is_some());
        assert!(find_duplicate("Customer", &existing).is_none());
    }

    #[test]
    fn versions_increment_and_odd_ones_restart() {
        assert_eq!(increment_version("v1"), "v2");
        assert_eq!(increment_version("v41"), "v42");
        assert_eq!(increment_version("draft"), "v2");
        assert_eq!(increment_version("v"), "v2");
    }

    #[test]
    fn the_output_is_deterministic_and_round_trips_through_the_owl_syntax() {
        let cands = OntologyCandidate { classes: vec![class("zeta"), class("alpha")], relations: vec![rel("alpha", "links", "zeta")], ..Default::default() };
        let (a, _) = engineer(&cands, &base(), "v1");
        let (b, _) = engineer(&cands, &base(), "v1");
        assert_eq!(a, b);
        let text = xiar_owl::functional::write(&a);
        assert_eq!(xiar_owl::functional::parse(&text).unwrap(), a);
        assert!(text.find("xiar:Agent").unwrap() < text.find("xiar:Zeta").unwrap(), "classes are sorted");
    }

    #[test]
    fn extra_axioms_in_the_base_are_kept() {
        let mut b = base();
        b.axioms.push(Axiom::SubClassOf(ClassExpr::Class(iri("Agent")), ClassExpr::Class(iri("Task"))));
        let (o, _) = engineer(&OntologyCandidate::default(), &b, "v3");
        assert!(o.axioms.contains(&Axiom::SubClassOf(ClassExpr::Class(iri("Agent")), ClassExpr::Class(iri("Task")))));
    }

    #[test]
    fn a_property_with_several_domains_keeps_the_alphabetically_first() {
        let classes: BTreeSet<String> = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
        let rels: BTreeSet<(String, String, String)> = [("B", "p", "C"), ("A", "p", "C"), ("A", "p", "B")].iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect();
        let o = build(&classes, &rels);
        let (_, r) = parts(&o);
        assert_eq!(r.iter().collect::<Vec<_>>(), [&("A".to_string(), "p".to_string(), "B".to_string())]);
    }
}
