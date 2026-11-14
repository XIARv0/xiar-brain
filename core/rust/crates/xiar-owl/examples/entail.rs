// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Developer tool: does the premise ontology entail every axiom of the conclusion ontology? (the check the W3C OWL test cases
//! are written for: `premises` + `conclusions` must entail, `premises` + `nonconclusions` must not).
//!
//! Each conclusion axiom is tested by adding its negation to the premise, using fresh individuals where the negation needs
//! witnesses, and asking whether the result is inconsistent. Axioms whose negation this tool cannot express (and anything that
//! mentions an anonymous individual, which a conclusion reads as "there is some individual") give `UNKNOWN`, never a guess.
//!
//! usage: entail <premise.ofn> <conclusion.ofn>
//! prints one line: ENTAILED, NOT_ENTAILED (with the first axiom that is not), UNKNOWN (with why) or ERROR

use xiar_owl::functional::parse;
use xiar_owl::model::*;
use xiar_owl::reasoner::{ReasonError, Reasoner};

fn fresh(n: usize) -> Iri {
    Iri::new(format!("urn:xiar-entail:{n}"))
}

/// The axioms to add to the premise so that, if the axiom were *not* entailed, they hold. `None` = cannot be expressed.
fn negation(a: &Axiom, next: &mut usize) -> Option<Vec<Axiom>> {
    let mut f = || {
        *next += 1;
        fresh(*next)
    };
    let thing = ClassExpr::Thing;
    Some(match a {
        Axiom::ClassAssertion(c, i) => vec![Axiom::ClassAssertion(ClassExpr::Not(Box::new(c.clone())), i.clone())],
        Axiom::SubClassOf(c, d) => {
            // some individual in c and not in d
            let x = f();
            vec![Axiom::ClassAssertion(c.clone(), x.clone()), Axiom::ClassAssertion(ClassExpr::Not(Box::new(d.clone())), x)]
        }
        Axiom::ObjectPropertyAssertion(p, a, b) => vec![Axiom::ClassAssertion(ClassExpr::All(p.clone(), Box::new(ClassExpr::Not(Box::new(ClassExpr::OneOf(vec![b.clone()]))))), a.clone())],
        Axiom::DataPropertyAssertion(p, a, l) => vec![Axiom::ClassAssertion(ClassExpr::DataAll(p.clone(), DataRange::Not(Box::new(DataRange::OneOf(vec![l.clone()])))), a.clone())],
        Axiom::SameIndividual(v) if v.len() == 2 => vec![Axiom::DifferentIndividuals(v.clone())],
        Axiom::DifferentIndividuals(v) if v.len() == 2 => vec![Axiom::SameIndividual(v.clone())],
        Axiom::DisjointClasses(v) if v.len() == 2 => {
            let x = f();
            vec![Axiom::ClassAssertion(v[0].clone(), x.clone()), Axiom::ClassAssertion(v[1].clone(), x)]
        }
        Axiom::ObjectPropertyDomain(p, c) => {
            let (x, y) = (f(), f());
            vec![Axiom::ObjectPropertyAssertion(p.clone(), x.clone(), y), Axiom::ClassAssertion(ClassExpr::Not(Box::new(c.clone())), x)]
        }
        Axiom::ObjectPropertyRange(p, c) => {
            let (x, y) = (f(), f());
            vec![Axiom::ObjectPropertyAssertion(p.clone(), x, y.clone()), Axiom::ClassAssertion(ClassExpr::Not(Box::new(c.clone())), y)]
        }
        Axiom::FunctionalObjectProperty(p) => {
            let (x, y, z) = (f(), f(), f());
            vec![Axiom::ObjectPropertyAssertion(p.clone(), x.clone(), y.clone()), Axiom::ObjectPropertyAssertion(p.clone(), x, z.clone()), Axiom::DifferentIndividuals(vec![y, z])]
        }
        Axiom::InverseFunctionalObjectProperty(p) => {
            let (x, y, z) = (f(), f(), f());
            vec![Axiom::ObjectPropertyAssertion(p.clone(), y.clone(), x.clone()), Axiom::ObjectPropertyAssertion(p.clone(), z.clone(), x), Axiom::DifferentIndividuals(vec![y, z])]
        }
        Axiom::TransitiveObjectProperty(p) => {
            let (x, y, z) = (f(), f(), f());
            vec![
                Axiom::ObjectPropertyAssertion(p.clone(), x.clone(), y.clone()),
                Axiom::ObjectPropertyAssertion(p.clone(), y, z.clone()),
                Axiom::ClassAssertion(ClassExpr::All(p.clone(), Box::new(ClassExpr::Not(Box::new(ClassExpr::OneOf(vec![z]))))), x),
            ]
        }
        Axiom::SymmetricObjectProperty(p) => {
            let (x, y) = (f(), f());
            vec![Axiom::ObjectPropertyAssertion(p.clone(), x.clone(), y.clone()), Axiom::ClassAssertion(ClassExpr::All(p.inverse(), Box::new(ClassExpr::Not(Box::new(ClassExpr::OneOf(vec![x.clone()]))))), y)]
        }
        Axiom::SubObjectPropertyOf(p, q) => {
            let (x, y) = (f(), f());
            vec![Axiom::ObjectPropertyAssertion(p.clone(), x.clone(), y.clone()), Axiom::ClassAssertion(ClassExpr::All(q.clone(), Box::new(ClassExpr::Not(Box::new(ClassExpr::OneOf(vec![y]))))), x)]
        }
        Axiom::InverseObjectProperties(p, q) => {
            let (x, y) = (f(), f());
            vec![
                Axiom::ObjectPropertyAssertion(ObjectProp::Named(p.clone()), x.clone(), y.clone()),
                Axiom::ClassAssertion(ClassExpr::All(ObjectProp::Named(q.clone()), Box::new(ClassExpr::Not(Box::new(ClassExpr::OneOf(vec![x]))))), y),
            ]
        }
        Axiom::DataPropertyDomain(p, c) => {
            let x = f();
            vec![Axiom::ClassAssertion(ClassExpr::DataSome(p.clone(), DataRange::Top), x.clone()), Axiom::ClassAssertion(ClassExpr::Not(Box::new(c.clone())), x)]
        }
        Axiom::DataPropertyRange(p, d) => {
            let x = f();
            vec![Axiom::ClassAssertion(ClassExpr::DataSome(p.clone(), DataRange::Not(Box::new(d.clone()))), x)]
        }
        Axiom::FunctionalDataProperty(p) => {
            let x = f();
            vec![Axiom::ClassAssertion(ClassExpr::DataMin(2, p.clone(), DataRange::Top), x)]
        }
        _ => {
            let _ = thing;
            return None;
        }
    })
}

/// The axioms an equivalence or a multi-way statement stands for, as pairs.
fn split(a: &Axiom) -> Vec<Axiom> {
    match a {
        Axiom::EquivalentClasses(v) if v.len() >= 2 => (1..v.len()).flat_map(|i| [Axiom::SubClassOf(v[0].clone(), v[i].clone()), Axiom::SubClassOf(v[i].clone(), v[0].clone())]).collect(),
        Axiom::EquivalentObjectProperties(v) if v.len() >= 2 => (1..v.len()).flat_map(|i| [Axiom::SubObjectPropertyOf(v[0].clone(), v[i].clone()), Axiom::SubObjectPropertyOf(v[i].clone(), v[0].clone())]).collect(),
        Axiom::DisjointClasses(v) if v.len() > 2 => (0..v.len()).flat_map(|i| ((i + 1)..v.len()).map(move |j| (i, j))).map(|(i, j)| Axiom::DisjointClasses(vec![v[i].clone(), v[j].clone()])).collect(),
        Axiom::DifferentIndividuals(v) if v.len() > 2 => (0..v.len()).flat_map(|i| ((i + 1)..v.len()).map(move |j| (i, j))).map(|(i, j)| Axiom::DifferentIndividuals(vec![v[i].clone(), v[j].clone()])).collect(),
        Axiom::SameIndividual(v) if v.len() > 2 => v.windows(2).map(|w| Axiom::SameIndividual(w.to_vec())).collect(),
        other => vec![other.clone()],
    }
}

fn mentions_anonymous(a: &Axiom) -> bool {
    format!("{a:?}").contains("urn:xiar-anon:")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let read = |p: &str| parse(&std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{p}: {e}")));
    let (premise, conclusion) = match (read(&args[1]), read(&args[2])) {
        (Ok(p), Ok(c)) => (p, c),
        (Err(e), _) | (_, Err(e)) => {
            println!("ERROR {e}");
            return;
        }
    };
    let mut next = 0;
    let mut unknown: Option<String> = None;
    for ax in conclusion.axioms.iter().flat_map(split) {
        if matches!(ax, Axiom::DeclareClass(_) | Axiom::DeclareObjectProperty(_) | Axiom::DeclareDataProperty(_) | Axiom::DeclareIndividual(_) | Axiom::AnnotationAssertion(..)) {
            continue;
        }
        if mentions_anonymous(&ax) {
            unknown.get_or_insert_with(|| "an anonymous individual in the conclusion".to_string());
            continue;
        }
        let Some(extra) = negation(&ax, &mut next) else {
            unknown.get_or_insert_with(|| format!("cannot negate {:?}", std::mem::discriminant(&ax)));
            continue;
        };
        let mut o = premise.clone();
        o.axioms.extend(extra);
        match Reasoner::new(&o).with_budget(2_000_000, 200_000).is_consistent() {
            Ok(false) => {}
            Ok(true) => {
                println!("NOT_ENTAILED {}", xiar_owl::functional::write(&Ontology { axioms: vec![ax], ..Ontology::default() }).replace('\n', " ").chars().take(160).collect::<String>());
                return;
            }
            Err(ReasonError::Inconsistent) => {}
            Err(e) => {
                println!("ERROR {e}");
                return;
            }
        }
    }
    match unknown {
        Some(why) => println!("UNKNOWN {why}"),
        None => println!("ENTAILED"),
    }
}
