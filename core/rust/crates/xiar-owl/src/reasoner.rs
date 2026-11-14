// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! A tableau reasoner for the XIAR OWL profile (the description logic SHIQ with ABox: role hierarchy, inverse and
//! transitive roles, qualified number restrictions, no unique-name assumption), extended with nominals and with data
//! properties over data ranges.
//!
//! *Nominals* (`{a}`): a node whose label holds `{a}` is merged into the node of the individual `a`. Nominals together with
//! inverse roles and number restrictions need the extra care of the SHOIQ **NN-rule**: if a nominal node `x` has `≤ n S.C` and a
//! tree node `y` with `C` points at it (so `y` is an `S`-neighbour of `x` that is not a child of `x`), then blocking could copy
//! `y` and give `x` more than `n` neighbours, and merging two such nodes from different branches would break the tree shape.
//! The textbook rule guesses `m ≤ n` fresh nominals for them. Here `y` itself is *promoted* to a root node, which is what the
//! guess achieves (roots are never blocked or copied, and merging into a root is always allowed): nothing is guessed, and any
//! model of the ontology can be read as a model of the promoted graph (the promoted node is just a new name for an element), so
//! completeness is kept. The number of such roots at `x` is capped by `n`, because `≤ n` forces merges between them.
//!
//! *Data*: a node's data constraints are collected in its label (`DataSome`, `DataAll`, cardinalities) and its asserted
//! values; once the object part is complete, [`crate::data`] decides whether each node's values can satisfy them all.
//!
//! Method: concepts are put in negation normal form; the TBox is *absorbed* where possible (`A ⊑ D` becomes a lazy
//! unfolding rule, domains and ranges deterministic rules; `D ⊑ A` with a complex `D` is *not* absorbed: lazy unfolding
//! of `¬A` is only complete if every node also decides `A` or `¬A`, so it stays an internalised disjunction) and the rest is internalised as a
//! concept every node must satisfy. The completion graph is expanded with the usual rules (⊓, ⊔, ∃, ∀ with transitivity,
//! ≥, ≤ with merging, choose) under dynamic **pairwise blocking**, which is what makes inverse roles plus number
//! restrictions terminate soundly. Non-deterministic rules use explicit backtracking over saved states.
//!
//! The reasoner never guesses: a step or node budget that runs out is an error ([`ReasonError::Budget`]), not a verdict.

use crate::model::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ReasonError {
    #[error("the ontology is inconsistent")]
    Inconsistent,
    #[error("reasoning exceeded its budget ({0})")]
    Budget(&'static str),
    #[error("not supported by this reasoner: {0}")]
    Unsupported(String),
}

type Res<T> = Result<T, ReasonError>;
type Role = ObjectProp;
type NodeId = usize;

pub const MAX_STEPS: usize = 200_000;
pub const MAX_NODES: usize = 200_000;
/// Default memory budget in graph cells (label entries, edges and data values of every distinct node the search holds, saved
/// branch points included). About 50 bytes each in practice (measured: 20 million cells held about 1 GB), so 100 million is roughly 5 GB: exceeding it is a [`ReasonError::Budget`].
pub const MAX_CELLS: usize = 100_000_000;

// ------------------------------------------------------------------ concepts in NNF
fn top() -> ClassExpr {
    ClassExpr::Thing
}

fn bottom() -> ClassExpr {
    ClassExpr::Nothing
}

pub fn nnf(c: &ClassExpr) -> ClassExpr {
    match c {
        ClassExpr::Not(x) => neg(x),
        ClassExpr::And(v) => ClassExpr::And(v.iter().map(nnf).collect()),
        ClassExpr::Or(v) => ClassExpr::Or(v.iter().map(nnf).collect()),
        ClassExpr::Some(r, x) => ClassExpr::Some(r.clone(), Box::new(nnf(x))),
        ClassExpr::All(r, x) => ClassExpr::All(r.clone(), Box::new(nnf(x))),
        ClassExpr::Min(0, _, _) => top(),
        ClassExpr::Min(n, r, x) => ClassExpr::Min(*n, r.clone(), Box::new(nnf(x))),
        ClassExpr::Max(n, r, x) => ClassExpr::Max(*n, r.clone(), Box::new(nnf(x))),
        ClassExpr::Exact(n, r, x) => ClassExpr::And(vec![nnf(&ClassExpr::Min(*n, r.clone(), x.clone())), ClassExpr::Max(*n, r.clone(), Box::new(nnf(x)))]),
        // a nominal with several members is a disjunction of single nominals; a single nominal is an atom
        ClassExpr::OneOf(v) if v.len() > 1 => ClassExpr::Or(v.iter().map(|i| ClassExpr::OneOf(vec![i.clone()])).collect()),
        ClassExpr::HasValue(r, i) => ClassExpr::Some(r.clone(), Box::new(ClassExpr::OneOf(vec![i.clone()]))),
        ClassExpr::DataMin(0, _, _) => top(),
        ClassExpr::DataExact(n, p, d) => ClassExpr::And(vec![ClassExpr::DataMin(*n, p.clone(), d.clone()), ClassExpr::DataMax(*n, p.clone(), d.clone())]),
        other => other.clone(),
    }
}

fn dneg(d: &DataRange) -> DataRange {
    match d {
        DataRange::Not(x) => (**x).clone(),
        other => DataRange::Not(Box::new(other.clone())),
    }
}

/// NNF of the complement of `c`.
pub fn neg(c: &ClassExpr) -> ClassExpr {
    match c {
        ClassExpr::Thing => bottom(),
        ClassExpr::Nothing => top(),
        ClassExpr::Class(_) => ClassExpr::Not(Box::new(c.clone())),
        ClassExpr::Not(x) => nnf(x),
        ClassExpr::And(v) => ClassExpr::Or(v.iter().map(neg).collect()),
        ClassExpr::Or(v) => ClassExpr::And(v.iter().map(neg).collect()),
        ClassExpr::Some(r, x) => ClassExpr::All(r.clone(), Box::new(neg(x))),
        ClassExpr::All(r, x) => ClassExpr::Some(r.clone(), Box::new(neg(x))),
        ClassExpr::Min(0, _, _) => bottom(),
        ClassExpr::Min(n, r, x) => ClassExpr::Max(n - 1, r.clone(), Box::new(nnf(x))),
        ClassExpr::Max(n, r, x) => ClassExpr::Min(n + 1, r.clone(), Box::new(nnf(x))),
        ClassExpr::Exact(n, r, x) => ClassExpr::Or(vec![neg(&ClassExpr::Min(*n, r.clone(), x.clone())), neg(&ClassExpr::Max(*n, r.clone(), x.clone()))]),
        ClassExpr::OneOf(v) if v.len() == 1 => ClassExpr::Not(Box::new(c.clone())),
        ClassExpr::OneOf(v) => ClassExpr::And(v.iter().map(|i| ClassExpr::Not(Box::new(ClassExpr::OneOf(vec![i.clone()])))).collect()),
        ClassExpr::HasValue(r, i) => ClassExpr::All(r.clone(), Box::new(ClassExpr::Not(Box::new(ClassExpr::OneOf(vec![i.clone()]))))),
        ClassExpr::HasSelf(_) | ClassExpr::AllState(..) => ClassExpr::Not(Box::new(c.clone())),
        ClassExpr::DataSome(p, d) => ClassExpr::DataAll(p.clone(), dneg(d)),
        ClassExpr::DataAll(p, d) => ClassExpr::DataSome(p.clone(), dneg(d)),
        ClassExpr::DataHasValue(p, l) => ClassExpr::DataAll(p.clone(), DataRange::Not(Box::new(DataRange::OneOf(vec![l.clone()])))),
        ClassExpr::DataMin(0, _, _) => bottom(),
        ClassExpr::DataMin(n, p, d) => ClassExpr::DataMax(n - 1, p.clone(), d.clone()),
        ClassExpr::DataMax(n, p, d) => ClassExpr::DataMin(n + 1, p.clone(), d.clone()),
        ClassExpr::DataExact(n, p, d) => ClassExpr::Or(vec![neg(&ClassExpr::DataMin(*n, p.clone(), d.clone())), neg(&ClassExpr::DataMax(*n, p.clone(), d.clone()))]),
    }
}

/// How expensive it is to commit to a disjunct, to order the alternatives of a disjunction: a negated name or a restriction that
/// only constrains first (saying a node is *not* something adds the least), then plain names, then anything that generates
/// nodes. A disjunction is as cheap as its cheapest alternative, a conjunction as dear as its dearest part.
fn branch_cost(c: &ClassExpr) -> u8 {
    match c {
        ClassExpr::Thing | ClassExpr::Nothing | ClassExpr::OneOf(_) => 0,
        ClassExpr::Not(x) if matches!(**x, ClassExpr::Class(_) | ClassExpr::OneOf(_)) => 0,
        ClassExpr::All(..) | ClassExpr::Max(..) => 1,
        ClassExpr::Class(_) => 2,
        ClassExpr::Or(v) => v.iter().map(branch_cost).min().unwrap_or(0),
        ClassExpr::And(v) => v.iter().map(branch_cost).max().unwrap_or(0),
        _ => 3,
    }
}

// ------------------------------------------------------------------ role hierarchy
#[derive(Default)]
struct Hier {
    sup: BTreeMap<Role, BTreeSet<Role>>,
    trans: BTreeSet<Role>,
    /// the targets of property chains (and their inverses): roles that are not simple
    complex: BTreeSet<Role>,
}

impl Hier {
    fn is_sub(&self, r: &Role, s: &Role) -> bool {
        r == s || self.sup.get(r).is_some_and(|x| x.contains(s))
    }
}

fn build_hierarchy(o: &Ontology) -> Hier {
    let mut direct: BTreeMap<Role, BTreeSet<Role>> = BTreeMap::new();
    let mut add = |p: &Role, q: &Role| {
        direct.entry(p.clone()).or_default().insert(q.clone());
        direct.entry(p.inverse()).or_default().insert(q.inverse());
    };
    let mut trans = BTreeSet::new();
    let mut complex = BTreeSet::new();
    for a in &o.axioms {
        match a {
            Axiom::SubPropertyChain(_, t) => {
                complex.insert(t.clone());
                complex.insert(t.inverse());
            }
            Axiom::SubObjectPropertyOf(p, q) => add(p, q),
            Axiom::EquivalentObjectProperties(v) => {
                for p in v {
                    for q in v {
                        if p != q {
                            add(p, q);
                        }
                    }
                }
            }
            Axiom::InverseObjectProperties(p, q) => {
                add(&Role::Named(p.clone()), &Role::Inverse(q.clone()));
                add(&Role::Inverse(q.clone()), &Role::Named(p.clone()));
            }
            Axiom::SymmetricObjectProperty(p) => {
                add(p, &p.inverse());
                add(&p.inverse(), p);
            }
            Axiom::TransitiveObjectProperty(p) => {
                trans.insert(p.clone());
                trans.insert(p.inverse());
            }
            _ => {}
        }
    }
    // reflexive-transitive closure
    let keys: Vec<Role> = direct.keys().cloned().collect();
    let mut sup: BTreeMap<Role, BTreeSet<Role>> = BTreeMap::new();
    for k in keys {
        let mut seen: BTreeSet<Role> = BTreeSet::new();
        let mut todo = vec![k.clone()];
        while let Some(r) = todo.pop() {
            for s in direct.get(&r).into_iter().flatten() {
                if seen.insert(s.clone()) {
                    todo.push(s.clone());
                }
            }
        }
        seen.insert(k.clone());
        sup.insert(k, seen);
    }
    Hier { sup, trans, complex }
}

fn collect_chains(o: &Ontology) -> Vec<(Vec<Role>, Role)> {
    o.axioms.iter().filter_map(|a| if let Axiom::SubPropertyChain(w, t) = a { Some((w.clone(), t.clone())) } else { None }).collect()
}

/// OWL 2 DL requires *simple* roles (no transitive role among their sub-roles) in cardinality restrictions and in
/// functional / inverse-functional axioms; without that restriction the logic is undecidable and a tableau is unsound.
/// Returns a description of the first violation.
pub fn check_simple_roles(o: &Ontology) -> Result<(), String> {
    let h = build_hierarchy(o);
    crate::chains::build(&h.sup, &collect_chains(o), &h.trans)?;
    let non_simple = |p: &Role| h.trans.iter().chain(&h.complex).any(|t| h.is_sub(t, p) || h.is_sub(t, &p.inverse()));
    fn cardinality_roles(c: &ClassExpr, out: &mut Vec<Role>) {
        match c {
            ClassExpr::And(v) | ClassExpr::Or(v) => v.iter().for_each(|x| cardinality_roles(x, out)),
            ClassExpr::Not(x) | ClassExpr::Some(_, x) | ClassExpr::All(_, x) | ClassExpr::AllState(_, _, x) => cardinality_roles(x, out),
            ClassExpr::Min(_, r, x) | ClassExpr::Max(_, r, x) | ClassExpr::Exact(_, r, x) => {
                out.push(r.clone());
                cardinality_roles(x, out);
            }
            ClassExpr::HasSelf(r) => out.push(r.clone()),
            ClassExpr::Thing | ClassExpr::Nothing | ClassExpr::Class(_) | ClassExpr::OneOf(_) | ClassExpr::HasValue(..) => {}
            ClassExpr::DataSome(..) | ClassExpr::DataAll(..) | ClassExpr::DataHasValue(..) | ClassExpr::DataMin(..) | ClassExpr::DataMax(..) | ClassExpr::DataExact(..) => {}
        }
    }
    let mut used: Vec<(Role, &'static str)> = Vec::new();
    for a in &o.axioms {
        let mut exprs: Vec<&ClassExpr> = Vec::new();
        match a {
            Axiom::SubClassOf(x, y) => exprs.extend([x, y]),
            Axiom::EquivalentClasses(v) | Axiom::DisjointClasses(v) => exprs.extend(v.iter()),
            Axiom::ObjectPropertyDomain(_, c) | Axiom::ObjectPropertyRange(_, c) | Axiom::DataPropertyDomain(_, c) | Axiom::ClassAssertion(c, _) => exprs.push(c),
            Axiom::FunctionalObjectProperty(p) | Axiom::InverseFunctionalObjectProperty(p) => used.push((p.clone(), "a functional axiom")),
            Axiom::IrreflexiveObjectProperty(p) | Axiom::AsymmetricObjectProperty(p) => used.push((p.clone(), "an irreflexive or asymmetric axiom")),
            Axiom::DisjointObjectProperties(v) => used.extend(v.iter().map(|p| (p.clone(), "a disjointness axiom"))),
            _ => {}
        }
        for e in exprs {
            let mut rs = Vec::new();
            cardinality_roles(e, &mut rs);
            used.extend(rs.into_iter().map(|r| (r, "a cardinality or self restriction")));
        }
    }
    for (r, what) in used {
        if non_simple(&r) {
            return Err(format!("property `{}` is transitive, the target of a property chain, or has such a sub-property, so it cannot be used in {what}", r.named().local()));
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ compiled TBox
#[derive(Default)]
struct Tbox {
    hier: Hier,
    /// `A ∈ L(x)` adds these
    unfold: BTreeMap<Iri, Vec<ClassExpr>>,
    /// `¬A ∈ L(x)` adds these (only for classes defined by a single equivalence that nothing else builds on, see `compile`)
    neg_unfold: BTreeMap<Iri, Vec<ClassExpr>>,
    /// the classes handled that way: the label of a node does not say whether the node is in them unless it was forced
    lazy: BTreeSet<Iri>,
    /// concepts every node must satisfy
    universal: Vec<ClassExpr>,
    /// facts about individuals that come from axioms with a nominal on the left (`{a} ⊑ D`)
    nominal_facts: Vec<(Iri, ClassExpr)>,
    /// if x has a neighbour via a sub-role of `R`, x satisfies `C`
    domains: Vec<(Role, ClassExpr)>,
    /// if x has a value for a sub-property of `p`, x satisfies `C`
    data_domains: Vec<(Iri, ClassExpr)>,
    /// every data property above a data property, itself included
    data_super: BTreeMap<Iri, BTreeSet<Iri>>,
    /// `HasKey` axioms: the class (in NNF) and the object and data key properties
    keys: Vec<(ClassExpr, Vec<Role>, Vec<Iri>)>,
    /// pairs of data properties that share no value (the sub-properties of the two are disjoint as well)
    data_disjoint: Vec<(Iri, Iri)>,
    /// the automata of the roles with property chains (see [`crate::chains`]), or why there are none
    chains: crate::chains::Chains,
    chain_error: Option<String>,
    /// every node has a `p`-edge to itself
    reflexive: Vec<Role>,
    /// a node with a `p`-edge to itself is a clash
    irreflexive: Vec<Role>,
    /// a pair of nodes joined by a sub-role of `p` and by a sub-role of `q` is a clash
    disjoint_roles: Vec<(Role, Role)>,
}

fn build_data_hierarchy(o: &Ontology) -> BTreeMap<Iri, BTreeSet<Iri>> {
    let mut direct: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
    for a in &o.axioms {
        match a {
            Axiom::SubDataPropertyOf(p, q) => {
                direct.entry(p.clone()).or_default().insert(q.clone());
            }
            Axiom::EquivalentDataProperties(v) => {
                for p in v {
                    for q in v {
                        if p != q {
                            direct.entry(p.clone()).or_default().insert(q.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut sup: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
    for p in o.data_property_names() {
        let mut seen: BTreeSet<Iri> = BTreeSet::new();
        let mut todo = vec![p.clone()];
        while let Some(x) = todo.pop() {
            if seen.insert(x.clone()) {
                todo.extend(direct.get(&x).into_iter().flatten().cloned());
            }
        }
        sup.insert(p, seen);
    }
    sup
}

fn absorb(t: &mut Tbox, c: &ClassExpr, d: &ClassExpr) {
    // c ⊑ d
    if let ClassExpr::And(ds) = d {
        // c ⊑ d1 ⊓ d2 is c ⊑ d1 and c ⊑ d2
        for x in ds {
            absorb(t, c, x);
        }
        return;
    }
    match c {
        // (c1 ⊔ c2) ⊑ d is c1 ⊑ d and c2 ⊑ d: no guess at every node
        ClassExpr::Or(parts) => {
            for p in parts {
                absorb(t, p, d);
            }
        }
        // {a} ⊑ d is the fact that a is a d
        ClassExpr::OneOf(v) => {
            for i in v {
                t.nominal_facts.push((i.clone(), nnf(d)));
            }
        }
        ClassExpr::Class(a) => t.unfold.entry(a.clone()).or_default().push(nnf(d)),
        ClassExpr::Thing => t.universal.push(nnf(d)),
        ClassExpr::And(parts) if parts.iter().any(|p| matches!(p, ClassExpr::Class(_))) => {
            let i = parts.iter().position(|p| matches!(p, ClassExpr::Class(_))).unwrap();
            let ClassExpr::Class(a) = &parts[i] else { unreachable!() };
            let rest: Vec<ClassExpr> = parts.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, p)| p.clone()).collect();
            let rest_c = if rest.len() == 1 { rest[0].clone() } else { ClassExpr::And(rest) };
            // A ⊓ R ⊑ D  ⟹  A ⊑ ¬R ⊔ D
            t.unfold.entry(a.clone()).or_default().push(nnf(&ClassExpr::Or(vec![neg(&rest_c), d.clone()])));
        }
        _ => match d {
            ClassExpr::Not(x) if matches!(**x, ClassExpr::Class(_)) => {
                // C ⊑ ¬A  ⟹  A ⊑ ¬C
                let ClassExpr::Class(a) = &**x else { unreachable!() };
                t.unfold.entry(a.clone()).or_default().push(neg(c));
            }
            _ => t.universal.push(nnf(&ClassExpr::Or(vec![neg(c), d.clone()]))),
        },
    }
}

/// Compile one axiom (everything but the definitions that `compile` decides to treat lazily).
/// `DisjointUnion(A C1 … Cn)` is `A ≡ C1 ⊔ … ⊔ Cn` plus pairwise disjointness. A negative property assertion `¬p(a, b)` is the class assertion `a : ∀p.¬{b}` (for a data property, `∀p.¬{value}`), which the
/// nominal and data machinery already decide.
fn normalize(o: &Ontology) -> Ontology {
    if !o.axioms.iter().any(|a| matches!(a, Axiom::NegativeObjectPropertyAssertion(..) | Axiom::NegativeDataPropertyAssertion(..) | Axiom::DisjointUnion(..))) {
        return o.clone();
    }
    let mut out = o.clone();
    out.axioms = std::mem::take(&mut out.axioms).into_iter().flat_map(|a| match a {
        Axiom::DisjointUnion(c, v) => vec![Axiom::EquivalentClasses(vec![ClassExpr::Class(c), ClassExpr::Or(v.clone())]), Axiom::DisjointClasses(v)],
        other => vec![other],
    }).collect();
    for a in &mut out.axioms {
        match a {
            Axiom::NegativeObjectPropertyAssertion(p, x, y) => *a = Axiom::ClassAssertion(ClassExpr::All(p.clone(), Box::new(ClassExpr::Not(Box::new(ClassExpr::OneOf(vec![y.clone()]))))), x.clone()),
            Axiom::NegativeDataPropertyAssertion(p, x, l) => *a = Axiom::ClassAssertion(ClassExpr::DataAll(p.clone(), DataRange::Not(Box::new(DataRange::OneOf(vec![l.clone()])))), x.clone()),
            _ => {}
        }
    }
    out
}

fn compile_axiom(t: &mut Tbox, a: &Axiom) {
    match a {
        Axiom::SubClassOf(c, d) => absorb(t, c, d),
        Axiom::EquivalentClasses(v) => {
            for i in 0..v.len() {
                for j in 0..v.len() {
                    if i != j {
                        absorb(t, &v[i], &v[j]);
                    }
                }
            }
        }
        Axiom::DisjointClasses(v) => {
            for i in 0..v.len() {
                for j in (i + 1)..v.len() {
                    absorb(t, &v[i], &ClassExpr::Not(Box::new(v[j].clone())));
                }
            }
        }
        // a role with property chains has `r`-paths that are not edges, so its domain is `∀r.⊥ ⊔ C`, which follows them
        Axiom::ObjectPropertyDomain(r, c) if t.chains.of.contains_key(r) => t.universal.push(ClassExpr::Or(vec![ClassExpr::All(r.clone(), Box::new(bottom())), nnf(c)])),
        Axiom::ObjectPropertyDomain(r, c) => t.domains.push((r.clone(), nnf(c))),
        Axiom::ObjectPropertyRange(r, c) => t.universal.push(nnf(&ClassExpr::All(r.clone(), Box::new(c.clone())))),
        Axiom::FunctionalObjectProperty(r) => t.universal.push(ClassExpr::Max(1, r.clone(), Box::new(top()))),
        Axiom::InverseFunctionalObjectProperty(r) => t.universal.push(ClassExpr::Max(1, r.inverse(), Box::new(top()))),
        Axiom::HasKey(c, ps, ds) => t.keys.push((nnf(c), ps.clone(), ds.clone())),
        Axiom::DisjointDataProperties(v) => {
            for i in 0..v.len() {
                for j in (i + 1)..v.len() {
                    t.data_disjoint.push((v[i].clone(), v[j].clone()));
                }
            }
        }
        Axiom::ReflexiveObjectProperty(r) => t.reflexive.push(r.clone()),
        Axiom::IrreflexiveObjectProperty(r) => t.irreflexive.push(r.clone()),
        Axiom::AsymmetricObjectProperty(r) => {
            t.irreflexive.push(r.clone());
            t.disjoint_roles.push((r.clone(), r.inverse()));
        }
        Axiom::DisjointObjectProperties(v) => {
            for i in 0..v.len() {
                for j in (i + 1)..v.len() {
                    t.disjoint_roles.push((v[i].clone(), v[j].clone()));
                }
            }
        }
        Axiom::DataPropertyDomain(p, c) => t.data_domains.push((p.clone(), nnf(c))),
        Axiom::DataPropertyRange(p, d) => t.universal.push(ClassExpr::DataAll(p.clone(), d.clone())),
        Axiom::FunctionalDataProperty(p) => t.universal.push(ClassExpr::DataMax(1, p.clone(), DataRange::Top)),
        _ => {}
    }
}

/// `A ≡ D` where `A` is a name: the only equivalence axioms with exactly two members, one of them a plain class.
fn definition_of(a: &Axiom) -> Option<(Iri, ClassExpr)> {
    let Axiom::EquivalentClasses(v) = a else { return None };
    if v.len() != 2 {
        return None;
    }
    match (&v[0], &v[1]) {
        (ClassExpr::Class(x), ClassExpr::Class(y)) => Some(if x <= y { (x.clone(), v[1].clone()) } else { (y.clone(), v[0].clone()) }),
        (ClassExpr::Class(x), d) | (d, ClassExpr::Class(x)) => Some((x.clone(), d.clone())),
        _ => None,
    }
}

fn compile(o: &Ontology) -> Tbox {
    // A defined class `A ≡ D` is normally compiled as `A ⊑ D` (unfold) plus the internalised `D ⊑ A`, which makes every node
    // guess `A` or `¬A` - hopeless on a chain of dozens of definitions. It can instead be unfolded both ways (`A` adds `D`,
    // `¬A` adds `¬D`, no guess) when nothing else makes "A is true" fire a rule, i.e. no other axiom is absorbed onto `A`; and
    // the definitions must not depend on each other in a cycle. Both are checked here, to a fixpoint.
    let defs: Vec<(usize, Iri, ClassExpr)> = o.axioms.iter().enumerate().filter_map(|(i, a)| definition_of(a).map(|(n, d)| (i, n, d))).collect();
    let mut count: BTreeMap<Iri, usize> = BTreeMap::new();
    for (_, n, _) in &defs {
        *count.entry(n.clone()).or_default() += 1;
    }
    let mut lazy: BTreeSet<Iri> = defs.iter().filter(|(_, n, _)| count[n] == 1).map(|(_, n, _)| n.clone()).collect();
    loop {
        // cycles among the lazy definitions
        let body: BTreeMap<&Iri, BTreeSet<Iri>> = defs
            .iter()
            .filter(|(_, n, _)| lazy.contains(n))
            .map(|(_, n, d)| {
                let mut s = BTreeSet::new();
                d.classes(&mut s);
                (n, s)
            })
            .collect();
        let mut on_cycle: BTreeSet<Iri> = BTreeSet::new();
        for start in body.keys() {
            let mut seen: BTreeSet<&Iri> = BTreeSet::new();
            let mut todo: Vec<&Iri> = body[start].iter().collect();
            while let Some(c) = todo.pop() {
                if c == *start {
                    on_cycle.insert((*start).clone());
                    break;
                }
                if seen.insert(c) {
                    if let Some(next) = body.get(c) {
                        todo.extend(next.iter());
                    }
                }
            }
        }
        // names that the other axioms absorb rules onto
        let mut probe = Tbox::default();
        for (i, a) in o.axioms.iter().enumerate() {
            if !defs.iter().any(|(j, n, _)| *j == i && lazy.contains(n)) {
                compile_axiom(&mut probe, a);
            }
        }
        let bad: BTreeSet<Iri> = lazy.iter().filter(|n| on_cycle.contains(*n) || probe.unfold.contains_key(*n)).cloned().collect();
        // a definition that is not lazy is compiled as usual and so adds its own `unfold` entry, which the next round sees
        if bad.is_empty() {
            break;
        }
        for b in bad {
            lazy.remove(&b);
        }
    }
    let mut t = Tbox { hier: build_hierarchy(o), data_super: build_data_hierarchy(o), ..Tbox::default() };
    match crate::chains::build(&t.hier.sup, &collect_chains(o), &t.hier.trans) {
        Ok(c) => t.chains = c,
        Err(e) => t.chain_error = Some(e),
    }
    for (i, a) in o.axioms.iter().enumerate() {
        match defs.iter().find(|(j, n, _)| *j == i && lazy.contains(n)) {
            Some((_, n, d)) => {
                t.unfold.entry(n.clone()).or_default().push(nnf(d));
                t.neg_unfold.entry(n.clone()).or_default().push(neg(d));
            }
            None => compile_axiom(&mut t, a),
        }
    }
    t.lazy = lazy;
    t
}

// ------------------------------------------------------------------ completion graph
/// The branch points (disjunction choices, merges) a fact depends on, as a sorted shared list. Empty = unconditional.
/// Cloning is a reference-count bump, which keeps the per-branch state copies cheap.
#[derive(Clone, Default, PartialEq, Eq)]
struct Deps(Option<Arc<Vec<u32>>>);

impl Deps {
    fn new() -> Self {
        Deps(None)
    }

    fn one(l: u32) -> Self {
        Deps(Some(Arc::new(vec![l])))
    }

    fn items(&self) -> &[u32] {
        self.0.as_deref().map_or(&[], |v| v.as_slice())
    }

    fn max(&self) -> Option<u32> {
        self.items().last().copied()
    }

    fn with(&self, l: u32) -> Deps {
        self.union(&Deps::one(l))
    }

    fn union(&self, other: &Deps) -> Deps {
        if other.items().is_empty() || self == other {
            return self.clone();
        }
        if self.items().is_empty() {
            return other.clone();
        }
        let mut v: Vec<u32> = self.items().iter().chain(other.items()).copied().collect();
        v.sort_unstable();
        v.dedup();
        Deps(Some(Arc::new(v)))
    }

    fn without(&self, l: u32) -> Deps {
        let v: Vec<u32> = self.items().iter().copied().filter(|x| *x != l).collect();
        if v.is_empty() { Deps(None) } else { Deps(Some(Arc::new(v))) }
    }
}

#[derive(Clone)]
struct Node {
    label: BTreeMap<ClassExpr, Deps>,
    /// hash of the label's concepts (order independent), so blocking compares labels in O(1) before comparing them fully
    lhash: u64,
    out: BTreeMap<(Role, NodeId), Deps>,
    /// asserted data values of an individual (data property, value)
    values: Vec<(Iri, Literal, Deps)>,
    parent: Option<NodeId>,
    alive: bool,
    root: bool,
}

fn concept_hash(c: &ClassExpr) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    c.hash(&mut h);
    h.finish()
}

/// The completion graph. Nodes are copy-on-write, so saving a state at a branch point costs one reference count per node
/// plus a copy of only those nodes that change afterwards.
#[derive(Clone, Default)]
struct State {
    nodes: Vec<Arc<Node>>,
    neq: BTreeMap<(NodeId, NodeId), Deps>,
    nondet: bool,
    /// nodes whose deterministic rules must run again
    dirty: BTreeSet<NodeId>,
    /// nodes that changed since they were last found quiet, which the choice, disjunction and generation rules look at
    /// (each with the moment it last changed, so the most recently changed are looked at first)
    pending: BTreeMap<NodeId, u64>,
    tick: u64,
    /// the nodes the choice, at-most and disjunction rules still have to look at: a node leaves it when they find nothing to do
    /// there, and comes back when it or a neighbour changes, so a step costs what changed and not the size of the graph
    rules_le: BTreeSet<NodeId>,
    /// (the same for the disjunction rule)
    rules_or: BTreeSet<NodeId>,
    /// blocked flags at the last evaluation, to wake nodes that just became unblocked
    was_blocked: Vec<bool>,
    /// refresh calls since the blocking was last worked out in full (it is recomputed lazily, see `refresh_blocked`)
    blk_age: usize,
    clash: Option<Deps>,
    /// merged-away node -> the node it was merged into and what that merge depends on, so an individual's node can be
    /// found after merges (a conclusion that relies on "this individual is that node" depends on the merges on the way)
    redirect: BTreeMap<NodeId, (NodeId, Deps)>,
    /// the node each individual started in; follow `redirect` for where it is now (nominals merge into it)
    home: Arc<BTreeMap<Iri, NodeId>>,
    /// the nodes of the individuals that class expressions refer to, and how many nodes the individuals took in all: nodes made
    /// later (the node under test, what is generated from it) are decided first, then the nominals, then the other individuals
    nominal_nodes: Arc<BTreeSet<NodeId>>,
    abox_len: usize,
    /// the highest branch level any fact in this graph can depend on
    max_level: u32,
}

/// Is there a set of `n` nodes among `cands` that are pairwise known to be different? (A clique search: a greedy pick
/// can miss one and make the ≥-rule generate forever.)
fn has_distinct_set(st: &State, cands: &[NodeId], n: usize, chosen: &mut Vec<NodeId>) -> bool {
    if chosen.len() >= n {
        return true;
    }
    for (i, &y) in cands.iter().enumerate() {
        if chosen.len() + (cands.len() - i) < n {
            return false;
        }
        if chosen.iter().all(|&z| st.different(y, z).is_some()) {
            chosen.push(y);
            if has_distinct_set(st, &cands[i + 1..], n, chosen) {
                chosen.pop();
                return true;
            }
            chosen.pop();
        }
    }
    false
}

fn pair(a: NodeId, b: NodeId) -> (NodeId, NodeId) {
    if a <= b { (a, b) } else { (b, a) }
}

impl State {
    /// The live node that `x` has become after merges.
    fn resolve(&self, x: NodeId) -> NodeId {
        self.resolve_deps(x).0
    }

    /// The live node that `x` has become, with the dependencies of the merges that led there.
    fn resolve_deps(&self, mut x: NodeId) -> (NodeId, Deps) {
        let mut deps = Deps::new();
        while let Some((y, d)) = self.redirect.get(&x) {
            deps = deps.union(d);
            x = *y;
        }
        (x, deps)
    }

    fn new_node(&mut self, parent: Option<NodeId>, root: bool, label: impl IntoIterator<Item = (ClassExpr, Deps)>) -> NodeId {
        self.nodes.push(Arc::new(Node { label: BTreeMap::new(), lhash: 0, out: BTreeMap::new(), values: Vec::new(), parent, alive: true, root }));
        let id = self.nodes.len() - 1;
        for (c, d) in label {
            self.add_concept(id, c, d);
        }
        self.dirty.insert(id);
        self.mark(id);
        id
    }

    /// `x` and everything next to it must be looked at again (a change at `x` can change what its neighbours need).
    fn touch(&mut self, x: NodeId) {
        self.dirty.insert(x);
        self.mark(x);
        let near: Vec<NodeId> = self.nodes[x].out.keys().map(|(_, y)| *y).collect();
        for y in near {
            self.mark(y);
        }
    }

    /// Note that `x` changed: its rules need looking at again, and it goes to the front of the queue.
    fn mark(&mut self, x: NodeId) {
        self.tick += 1;
        self.pending.insert(x, self.tick);
        self.rules_le.insert(x);
        self.rules_or.insert(x);
    }

    /// Turn a tree node into a root node (see the module documentation on the NN-rule).
    fn promote(&mut self, y: NodeId) {
        let n = self.node_mut(y);
        n.root = true;
        n.parent = None;
        self.touch(y);
        self.blk_age = usize::MAX / 2; // what is blocked changes: work it out again at the next look
    }

    fn add_value(&mut self, x: NodeId, p: Iri, l: Literal, deps: Deps) {
        if !self.nodes[x].values.iter().any(|(q, m, _)| *q == p && *m == l) {
            self.node_mut(x).values.push((p, l, deps));
            self.touch(x);
        }
    }

    fn node_mut(&mut self, x: NodeId) -> &mut Node {
        Arc::make_mut(&mut self.nodes[x])
    }

    fn add_edge(&mut self, x: NodeId, r: &Role, y: NodeId, deps: &Deps) {
        self.node_mut(x).out.entry((r.clone(), y)).or_insert_with(|| deps.clone());
        self.node_mut(y).out.entry((r.inverse(), x)).or_insert_with(|| deps.clone());
        self.touch(x);
        self.touch(y);
    }

    /// Add `c` to the label of `x`. A direct contradiction is recorded as the state's clash, with its dependencies.
    fn add_concept(&mut self, x: NodeId, c: ClassExpr, deps: Deps) -> bool {
        if c == ClassExpr::Thing || self.nodes[x].label.contains_key(&c) {
            return false;
        }
        let clash = match &c {
            ClassExpr::Nothing => Some(deps.clone()),
            ClassExpr::Class(_) | ClassExpr::OneOf(_) => self.nodes[x].label.get(&ClassExpr::Not(Box::new(c.clone()))).map(|d| deps.union(d)),
            ClassExpr::Not(inner) => self.nodes[x].label.get(&**inner).map(|d| deps.union(d)),
            _ => None,
        };
        if let Some(d) = clash {
            if self.clash.is_none() {
                self.clash = Some(d);
            }
        }
        let h = concept_hash(&c);
        let n = self.node_mut(x);
        n.lhash ^= h;
        n.label.insert(c, deps);
        self.touch(x);
        true
    }

    fn has(&self, x: NodeId, c: &ClassExpr) -> bool {
        self.nodes[x].label.contains_key(c)
    }

    fn dep(&self, x: NodeId, c: &ClassExpr) -> Deps {
        self.nodes[x].label.get(c).cloned().unwrap_or_default()
    }

    fn different(&self, a: NodeId, b: NodeId) -> Option<&Deps> {
        self.neq.get(&pair(a, b))
    }

    /// Merge `from` into `into`, everything moved depending additionally on `deps`. `Err` carries the clash dependencies.
    fn merge(&mut self, from: NodeId, into: NodeId, deps: &Deps) -> Result<(), Deps> {
        if from == into {
            return Ok(());
        }
        if let Some(d) = self.different(from, into) {
            return Err(d.union(deps));
        }
        let edges: Vec<((Role, NodeId), Deps)> = self.nodes[from].out.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for ((r, w), d) in edges {
            self.node_mut(w).out.remove(&(r.inverse(), from));
            self.node_mut(from).out.remove(&(r.clone(), w));
            let w2 = if w == from { into } else { w };
            self.add_edge(into, &r, w2, &d.union(deps));
        }
        let vals = std::mem::take(&mut self.node_mut(from).values);
        for (p, l, d) in vals {
            self.add_value(into, p, l, d.union(deps));
        }
        let lab = std::mem::take(&mut self.node_mut(from).label);
        self.node_mut(from).lhash = 0;
        for (c, d) in lab {
            self.add_concept(into, c, d.union(deps));
        }
        let pairs: Vec<((NodeId, NodeId), Deps)> = self.neq.iter().filter(|((a, b), _)| *a == from || *b == from).map(|(k, v)| (*k, v.clone())).collect();
        for ((a, b), d) in pairs {
            self.neq.remove(&(a, b));
            let (a2, b2) = (if a == from { into } else { a }, if b == from { into } else { b });
            let nd = d.union(deps);
            if a2 == b2 {
                return Err(nd);
            }
            self.neq.entry(pair(a2, b2)).or_insert(nd);
        }
        for i in 0..self.nodes.len() {
            if self.nodes[i].parent == Some(from) {
                self.node_mut(i).parent = Some(into);
            }
        }
        if self.nodes[into].parent == Some(from) {
            let grand = self.nodes[from].parent;
            self.node_mut(into).parent = grand;
        }
        let f = self.node_mut(from);
        f.alive = false;
        f.parent = None;
        self.redirect.insert(from, (into, deps.clone()));
        self.dirty.remove(&from);
        self.pending.remove(&from);
        self.rules_le.remove(&from);
        self.rules_or.remove(&from);
        self.touch(into);
        Ok(())
    }
}

#[derive(Clone)]
enum Choice {
    /// add the concept to the node; the deps are those of the rule's premise
    Add(NodeId, ClassExpr, Deps),
    /// add several concepts (a disjunct plus the negations of the disjuncts tried before it)
    AddAll(NodeId, Vec<ClassExpr>, Deps),
    Merge(NodeId, NodeId, Deps),
}

enum Step {
    Fail(ReasonError),
    Clash(Deps),
    Applied,
    Branch(Vec<Choice>),
    Complete,
}

#[derive(Clone)]
struct Entry {
    snap: State,
    alts: Vec<Choice>,
    /// what the failed alternatives depended on (besides this branch point itself)
    acc: Deps,
    /// the concept the alternative now in force adds, so that its failure can be remembered (see `Reasoner::failures`)
    cur: Option<ClassExpr>,
    /// for a disjunction: the node and the disjunction, so the alternative that finally worked can be remembered
    site: Option<(NodeId, ClassExpr)>,
}

fn choice_key(c: &Choice) -> Option<ClassExpr> {
    match c {
        Choice::Add(_, d, _) => Some(d.clone()),
        Choice::AddAll(_, ds, _) => ds.first().cloned(),
        Choice::Merge(..) => None,
    }
}

pub struct Reasoner {
    /// work done by the last `run` (steps, nodes, branch points), for diagnosing slow ontologies
    pub last_run: std::cell::Cell<(usize, usize, usize)>,
    /// the complete graph of the whole ontology (consistency and instance types), computed once
    base: std::cell::OnceCell<Result<Option<AboxModel>, ReasonError>>,
    tops: std::cell::OnceCell<Result<BTreeSet<Iri>, ReasonError>>,
    onto: Ontology,
    tbox: Tbox,
    named: BTreeSet<Iri>,
    max_steps: usize,
    max_nodes: usize,
    max_cells: usize,
    /// the ontology uses nominals: every concept test then starts from the individuals' nodes
    nominals: bool,
    /// the ontology has data constraints, so complete graphs get the data check
    has_data: bool,
    /// a combination this reasoner does not decide (see the module documentation), reported instead of answered
    data_cache: std::cell::RefCell<std::collections::HashMap<String, bool>>,
    /// print the first branch points and clashes of each run to stderr (set `XIAR_OWL_TRACE=<steps>`), for diagnosing slow ontologies
    trace: Option<usize>,
    /// which nodes the choice rules look at first: the ones that changed last (a search under assumptions about a particular
    /// individual) or the oldest (the other searches)
    recent: std::cell::Cell<bool>,
    /// the strategy that settled the last class test (a hint for the next)
    hint: std::cell::Cell<usize>,
    /// the step limit of the round that settled the last class test: neighbouring questions are about as hard, so the next one
    /// starts there instead of paying for the smaller rounds again
    start_cap: std::cell::Cell<usize>,
    /// phase saving and failure counts are updated only while the whole ontology is searched for its first model: what a later
    /// question's model happened to choose says little about the next question
    learn: std::cell::Cell<bool>,
    /// how often an alternative of a disjunction has failed so far (across all runs of this reasoner): alternatives that keep
    /// failing are tried later, which only changes the order of the search, never its answer
    failures: std::cell::RefCell<std::collections::HashMap<ClassExpr, u32>>,
    /// phase saving: the alternative of a disjunction at a node that led to a complete model last time. It is tried first the
    /// next time (a later test usually needs the same choices about the individuals), which saves redoing all the guesses.
    phase: std::cell::RefCell<std::collections::HashMap<(NodeId, ClassExpr), ClassExpr>>,
    site: std::cell::RefCell<Option<(NodeId, ClassExpr)>>,
}

/// A complete graph for the whole ontology, the node each individual started in, and the choices (with their untried
/// alternatives) that led to it, so that the search can be resumed under extra assumptions.
type AboxModel = (State, BTreeMap<Iri, NodeId>, Vec<Arc<Entry>>);

fn data_constructs(c: &ClassExpr) -> bool {
    match c {
        ClassExpr::DataSome(..) | ClassExpr::DataAll(..) | ClassExpr::DataHasValue(..) | ClassExpr::DataMin(..) | ClassExpr::DataMax(..) | ClassExpr::DataExact(..) => true,
        ClassExpr::And(v) | ClassExpr::Or(v) => v.iter().any(data_constructs),
        ClassExpr::Not(x) | ClassExpr::Some(_, x) | ClassExpr::All(_, x) | ClassExpr::Min(_, _, x) | ClassExpr::Max(_, _, x) | ClassExpr::Exact(_, _, x) => data_constructs(x),
        _ => false,
    }
}

impl Reasoner {
    pub fn new(o: &Ontology) -> Self {
        let normalized = normalize(o);
        let o = &normalized;
        let has_data = o.axioms.iter().any(|a| matches!(a, Axiom::DataPropertyRange(..) | Axiom::FunctionalDataProperty(_) | Axiom::DataPropertyAssertion(..)) || a.class_exprs().into_iter().any(data_constructs));
        Self {
            last_run: Default::default(),
            base: Default::default(),
            tops: Default::default(),
            tbox: compile(o),
            named: o.class_names(),
            onto: o.clone(),
            max_steps: MAX_STEPS,
            max_nodes: MAX_NODES,
            max_cells: MAX_CELLS,
            nominals: o.uses_nominals(),
            has_data,
            data_cache: Default::default(),
            trace: std::env::var("XIAR_OWL_TRACE").ok().map(|v| v.parse().unwrap_or(400)),
            failures: Default::default(),
            recent: Default::default(),
            hint: Default::default(),
            start_cap: Default::default(),
            learn: Default::default(),
            phase: Default::default(),
            site: Default::default(),
        }
    }

    /// Lower (or raise) the work limits of one reasoning call. Exceeding them is a [`ReasonError::Budget`], never a verdict.
    pub fn with_budget(mut self, max_steps: usize, max_nodes: usize) -> Self {
        self.max_steps = max_steps;
        self.max_nodes = max_nodes;
        self
    }

    /// Lower (or raise) the memory budget, counted in graph cells (see [`MAX_CELLS`]).
    pub fn with_memory_budget(mut self, max_cells: usize) -> Self {
        self.max_cells = max_cells;
        self
    }

    /// Cells held by the current graph and by every saved branch point, counting each shared node once.
    fn live_cells(st: &State, stack: &[Arc<Entry>]) -> usize {
        // a saved branch point shares its nodes but owns a deep copy of the maps beside them (pairwise inequalities above all:
        // a count of 600 makes about 180,000 of them in every saved state), so those are counted once per state
        let own = |s: &State| s.nodes.len() + s.neq.len() + s.dirty.len() + s.pending.len() + s.rules_le.len() + s.rules_or.len() + s.was_blocked.len() + s.redirect.len();
        let mut seen: std::collections::HashSet<*const Node> = std::collections::HashSet::new();
        let mut cells = own(st) + stack.iter().map(|e| own(&e.snap)).sum::<usize>();
        for n in st.nodes.iter().chain(stack.iter().flat_map(|e| e.snap.nodes.iter())) {
            if seen.insert(Arc::as_ptr(n)) {
                cells += 1 + n.label.len() + n.out.len() + n.values.len();
            }
        }
        cells
    }

    /// `S`-neighbours of `x` with the dependencies of the connecting edge.
    fn neighbours(&self, st: &State, x: NodeId, s: &Role) -> Vec<(NodeId, Deps)> {
        let mut seen: BTreeMap<NodeId, Deps> = BTreeMap::new();
        for ((r, y), d) in &st.nodes[x].out {
            if st.nodes[*y].alive && self.tbox.hier.is_sub(r, s) {
                seen.entry(*y).or_insert_with(|| d.clone());
            }
        }
        seen.into_iter().collect()
    }

    fn initial_label(&self, extra: impl IntoIterator<Item = (ClassExpr, Deps)>) -> Vec<(ClassExpr, Deps)> {
        let mut l: Vec<(ClassExpr, Deps)> = self.tbox.universal.iter().map(|c| (c.clone(), Deps::new())).collect();
        l.extend(extra);
        l
    }

    // ---- blocking
    /// Pairwise "anywhere" blocking: a tree node is blocked when an *older*, unblocked tree node has the same label, the
    /// same parent label and the same edge to its parent (so the pair behaves identically below), or when its parent is
    /// blocked. Candidates are found through a table keyed by the two label hashes, so one pass costs O(nodes).
    fn blocked(&self, st: &State) -> Vec<bool> {
        let n = st.nodes.len();
        let mut b = vec![false; n];
        let edge_set = |a: NodeId, c: NodeId| -> BTreeSet<&Role> { st.nodes[a].out.keys().filter(|(_, y)| *y == c).map(|(r, _)| r).collect() };
        let same_label = |a: NodeId, c: NodeId| st.nodes[a].lhash == st.nodes[c].lhash && st.nodes[a].label.len() == st.nodes[c].label.len() && st.nodes[a].label.keys().eq(st.nodes[c].label.keys());
        let mut seen: std::collections::HashMap<(u64, u64), Vec<NodeId>> = std::collections::HashMap::new();
        for x in 0..n {
            let node = &st.nodes[x];
            if !node.alive || node.root {
                continue;
            }
            let Some(xp) = node.parent else { continue };
            if b[xp] {
                b[x] = true;
                continue;
            }
            let key = (node.lhash, st.nodes[xp].lhash);
            if let Some(cands) = seen.get(&key) {
                for &y in cands {
                    let yp = st.nodes[y].parent.expect("candidates are tree nodes");
                    if same_label(x, y) && same_label(xp, yp) && edge_set(yp, y) == edge_set(xp, x) {
                        b[x] = true;
                        break;
                    }
                }
            }
            if !b[x] {
                seen.entry(key).or_default().push(x);
            }
        }
        b
    }

    /// Blocked flags, waking nodes that were blocked at the last evaluation and are not any more.
    ///
    /// Working blocking out costs a pass over every node, so it is redone only when the graph has grown or changed enough
    /// (about every `nodes / 16` calls), or when `force` is set. Between those moments the previous flags are reused and a new
    /// node counts as unblocked. A stale flag can only cost extra work (an expansion that blocking would have avoided) or delay
    /// a rule at a node that is no longer blocked; `step` forces a full recomputation before it declares the graph complete,
    /// which wakes any such node, so the answer is the same.
    fn refresh_blocked(&self, st: &mut State, force: bool) -> Vec<bool> {
        let n = st.nodes.len();
        if !force && !st.was_blocked.is_empty() && st.blk_age < (n / 16).max(32) {
            st.blk_age += 1;
            let mut b = st.was_blocked.clone();
            b.resize(n, false);
            return b;
        }
        st.blk_age = 0;
        let b = self.blocked(st);
        for (i, now) in b.iter().enumerate() {
            if !now && st.was_blocked.get(i).copied().unwrap_or(false) {
                st.dirty.insert(i);
                st.mark(i);
            }
        }
        st.was_blocked = b.clone();
        b
    }

    /// Apply the deterministic rules to the nodes whose labels or edges changed, in rounds, until nothing changes.
    /// Returns the blocked flags that hold at the end, or the clash dependencies if a clash is found.
    fn deterministic(&self, st: &mut State) -> Result<Vec<bool>, Deps> {
        loop {
            if let Some(d) = &st.clash {
                return Err(d.clone());
            }
            let blocked = self.refresh_blocked(st, false);
            if st.dirty.is_empty() {
                if !self.tbox.keys.is_empty() && self.apply_keys(st) {
                    continue;
                }
                return Ok(blocked);
            }
            let batch = std::mem::take(&mut st.dirty);
            for x in batch {
                if st.nodes[x].alive {
                    self.role_axioms(st, x);
                }
                if !st.nodes[x].alive || blocked[x] {
                    continue;
                }
                let mut acts: Vec<(NodeId, ClassExpr, Deps)> = Vec::new();
                let mut merges: Vec<(NodeId, NodeId, Deps)> = Vec::new();
                let mut loops: Vec<(Role, Deps)> = Vec::new();
                for (c, cd) in &st.nodes[x].label {
                    match c {
                        // `∃r.Self`: the node is its own r-neighbour
                        ClassExpr::HasSelf(r) => {
                            if !st.nodes[x].out.contains_key(&(r.clone(), x)) {
                                loops.push((r.clone(), cd.clone()));
                            }
                        }
                        // `¬∃r.Self`: no loop through r or a sub-role of r
                        ClassExpr::Not(inner) if matches!(**inner, ClassExpr::HasSelf(_)) => {
                            let ClassExpr::HasSelf(r) = &**inner else { unreachable!() };
                            let hit = st.nodes[x].out.iter().find(|((s, y), _)| *y == x && (self.tbox.hier.is_sub(s, r) || self.tbox.hier.is_sub(s, &r.inverse())));
                            if let Some((_, ed)) = hit {
                                if st.clash.is_none() {
                                    st.clash = Some(cd.union(ed));
                                }
                            }
                        }
                        ClassExpr::OneOf(v) if v.len() == 1 => {
                            // the nominal rule: a node that is `{a}` is the individual `a`
                            if let Some(&h) = st.home.get(&v[0]) {
                                let (h, hd) = st.resolve_deps(h);
                                if h != x {
                                    merges.push((x, h, cd.union(&hd)));
                                }
                            }
                        }
                        ClassExpr::And(v) => {
                            acts.extend(v.iter().filter(|p| !st.has(x, p)).map(|p| (x, p.clone(), cd.clone())));
                        }
                        ClassExpr::Or(v) if !v.contains(&ClassExpr::Thing) && !v.iter().any(|d| st.has(x, d)) => {
                            // unit propagation: a disjunction whose other alternatives are all refuted by the label forces the last one
                            let mut open: Vec<&ClassExpr> = Vec::new();
                            let mut why = cd.clone();
                            for d in v {
                                match st.nodes[x].label.get(&neg(d)) {
                                    Some(nd) => why = why.union(nd),
                                    None => open.push(d),
                                }
                            }
                            match open.len() {
                                0 => {
                                    if st.clash.is_none() {
                                        st.clash = Some(why);
                                    }
                                }
                                1 => acts.push((x, open[0].clone(), why)),
                                _ => {}
                            }
                        }
                        ClassExpr::Not(inner) if matches!(**inner, ClassExpr::Class(_)) => {
                            let ClassExpr::Class(a) = &**inner else { unreachable!() };
                            acts.extend(self.tbox.neg_unfold.get(a).into_iter().flatten().filter(|d| !st.has(x, d)).map(|d| (x, d.clone(), cd.clone())));
                        }
                        ClassExpr::Class(a) => {
                            acts.extend(self.tbox.unfold.get(a).into_iter().flatten().filter(|d| !st.has(x, d)).map(|d| (x, d.clone(), cd.clone())));
                        }
                        // `∀B^q.C`: move along every edge the automaton can take from state `q`
                        ClassExpr::AllState(a, q, d) => {
                            let auto = &self.tbox.chains.autos[*a as usize];
                            for ((r, y), ed) in &st.nodes[x].out {
                                if !st.nodes[*y].alive {
                                    continue;
                                }
                                for (lab, q2) in &auto.moves[*q as usize] {
                                    if !self.tbox.hier.is_sub(r, lab) {
                                        continue;
                                    }
                                    let why = cd.union(ed);
                                    if auto.accepting[*q2 as usize] && !st.has(*y, d) {
                                        acts.push((*y, (**d).clone(), why.clone()));
                                    }
                                    if !auto.moves[*q2 as usize].is_empty() {
                                        let next = ClassExpr::AllState(*a, *q2, d.clone());
                                        if !st.has(*y, &next) {
                                            acts.push((*y, next, why));
                                        }
                                    }
                                }
                            }
                        }
                        ClassExpr::All(s, d) => {
                            if let Some(&a) = self.tbox.chains.of.get(s) {
                                let start = ClassExpr::AllState(a, 0, d.clone());
                                if !st.has(x, &start) {
                                    acts.push((x, start, cd.clone()));
                                }
                            }
                            for (y, ed) in self.neighbours(st, x, s) {
                                if !st.has(y, d) {
                                    acts.push((y, (**d).clone(), cd.union(&ed)));
                                }
                            }
                            // transitive sub-roles of s propagate the restriction
                            for r in &self.tbox.hier.trans {
                                if self.tbox.hier.is_sub(r, s) {
                                    let again = ClassExpr::All(r.clone(), d.clone());
                                    for (y, ed) in self.neighbours(st, x, r) {
                                        if !st.has(y, &again) {
                                            acts.push((y, again.clone(), cd.union(&ed)));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                for (r, d) in &self.tbox.domains {
                    if st.has(x, d) {
                        continue;
                    }
                    let witness = st.nodes[x].out.iter().find(|((s, y), _)| st.nodes[*y].alive && self.tbox.hier.is_sub(s, r)).map(|(_, ed)| ed.clone());
                    if let Some(ed) = witness {
                        acts.push((x, d.clone(), ed));
                    }
                }
                for (p, c) in &self.tbox.data_domains {
                    if !st.has(x, c) {
                        if let Some(d) = self.uses_data_property(st, x, p) {
                            acts.push((x, c.clone(), d));
                        }
                    }
                }
                for (r, d) in loops {
                    st.add_edge(x, &r, x, &d);
                }
                for (n, c, d) in acts {
                    st.add_concept(n, c, d);
                }
                for (from, into, d) in merges {
                    if st.nodes[from].alive && st.nodes[into].alive && from != into {
                        if let Err(c) = st.merge(from, into, &d) {
                            if st.clash.is_none() {
                                st.clash = Some(c);
                            }
                            break;
                        }
                    }
                }
                if !st.nodes[x].alive {
                    continue;
                }
                self.numeric_clash(st, x);
            }
        }
    }

    /// `HasKey`: two named individuals that are instances of the key class and agree on every key property are the same
    /// individual. Returns whether the graph changed (a merge, or a clash when the two are known to be different).
    fn apply_keys(&self, st: &mut State) -> bool {
        let mut named: Vec<(NodeId, Deps)> = Vec::new();
        for &h in st.home.values() {
            let (n, d) = st.resolve_deps(h);
            if st.nodes[n].alive && !named.iter().any(|(m, _)| *m == n) {
                named.push((n, d));
            }
        }
        let is_named = |z: NodeId| named.iter().any(|(m, _)| *m == z);
        for (c, ps, ds) in &self.tbox.keys {
            for a in 0..named.len() {
                let Some(cx) = st.nodes[named[a].0].label.get(c) else { continue };
                for b in (a + 1)..named.len() {
                    let (x, y) = (named[a].0, named[b].0);
                    let Some(cy) = st.nodes[y].label.get(c) else { continue };
                    let mut why = named[a].1.union(&named[b].1).union(cx).union(cy);
                    let mut agree = true;
                    for p in ps {
                        let common = st.nodes[x].out.iter().find_map(|((r, z), ex)| {
                            if !self.tbox.hier.is_sub(r, p) || !is_named(*z) || !st.nodes[*z].alive {
                                return None;
                            }
                            st.nodes[y].out.iter().find(|((r2, z2), _)| z2 == z && self.tbox.hier.is_sub(r2, p)).map(|(_, ey)| ex.union(ey))
                        });
                        match common {
                            Some(d) => why = why.union(&d),
                            None => {
                                agree = false;
                                break;
                            }
                        }
                    }
                    if agree {
                        for d in ds {
                            let shares = st.nodes[x].values.iter().find_map(|(q, l, dl)| {
                                if !self.tbox.data_super.get(q).is_some_and(|s| s.contains(d)) {
                                    return None;
                                }
                                let v = crate::data::value_of(l).ok()?;
                                st.nodes[y].values.iter().find(|(q2, l2, _)| self.tbox.data_super.get(q2).is_some_and(|s| s.contains(d)) && crate::data::value_of(l2).ok().as_ref() == Some(&v)).map(|(_, _, dy)| dl.union(dy))
                            });
                            match shares {
                                Some(dd) => why = why.union(&dd),
                                None => {
                                    agree = false;
                                    break;
                                }
                            }
                        }
                    }
                    if !agree {
                        continue;
                    }
                    match st.merge(y, x, &why) {
                        Ok(()) => {}
                        Err(d) => {
                            if st.clash.is_none() {
                                st.clash = Some(d);
                            }
                        }
                    }
                    return true;
                }
            }
        }
        false
    }

    /// The axioms about properties themselves: a reflexive property loops on every node, and an irreflexive, asymmetric or
    /// disjoint one must not be used where it is forbidden.
    fn role_axioms(&self, st: &mut State, x: NodeId) {
        let t = &self.tbox;
        if t.reflexive.is_empty() && t.irreflexive.is_empty() && t.disjoint_roles.is_empty() {
            return;
        }
        let none = Deps::new();
        for r in &t.reflexive {
            if !st.nodes[x].out.contains_key(&(r.clone(), x)) {
                st.add_edge(x, r, x, &none);
            }
        }
        if st.clash.is_some() {
            return;
        }
        let edges: Vec<(Role, NodeId, Deps)> = st.nodes[x].out.iter().filter(|((_, y), _)| st.nodes[*y].alive).map(|((r, y), d)| (r.clone(), *y, d.clone())).collect();
        for (r, y, d) in &edges {
            if *y == x {
                for p in &t.irreflexive {
                    if t.hier.is_sub(r, p) || t.hier.is_sub(r, &p.inverse()) {
                        st.clash = Some(d.clone());
                        return;
                    }
                }
            }
            for (p, q) in &t.disjoint_roles {
                if !t.hier.is_sub(r, p) {
                    continue;
                }
                for (s, y2, d2) in &edges {
                    if y2 == y && t.hier.is_sub(s, q) {
                        st.clash = Some(d.union(d2));
                        return;
                    }
                }
            }
        }
    }

    /// Does `x` have, or need, a value for `p` or one of its sub-properties? The dependencies of the evidence.
    fn uses_data_property(&self, st: &State, x: NodeId, p: &Iri) -> Option<Deps> {
        let below = |q: &Iri| q == p || self.tbox.data_super.get(q).is_some_and(|s| s.contains(p));
        for (q, _, d) in &st.nodes[x].values {
            if below(q) {
                return Some(d.clone());
            }
        }
        for (c, cd) in &st.nodes[x].label {
            match c {
                ClassExpr::DataSome(q, _) | ClassExpr::DataHasValue(q, _) | ClassExpr::DataMin(1.., q, _) if below(q) => return Some(cd.clone()),
                _ => {}
            }
        }
        None
    }

    /// The data check on a complete graph: can every node's data values satisfy its constraints? `Some(deps)` is a clash.
    fn data_check(&self, st: &State, blocked: &[bool]) -> Res<Option<Deps>> {
        for x in 0..st.nodes.len() {
            let node = &st.nodes[x];
            if !node.alive || blocked.get(x).copied().unwrap_or(false) {
                continue;
            }
            let mut deps = Deps::new();
            let mut alls: BTreeMap<&Iri, Vec<&DataRange>> = BTreeMap::new();
            let mut exist: BTreeMap<&Iri, Vec<(u32, DataRange)>> = BTreeMap::new();
            let mut maxes: BTreeMap<&Iri, Vec<(u32, &DataRange)>> = BTreeMap::new();
            let mut vals: BTreeMap<&Iri, Vec<Literal>> = BTreeMap::new();
            for (c, cd) in &node.label {
                match c {
                    ClassExpr::DataAll(p, d) => {
                        alls.entry(p).or_default().push(d);
                        deps = deps.union(cd);
                    }
                    ClassExpr::DataSome(p, d) => {
                        exist.entry(p).or_default().push((1, d.clone()));
                        deps = deps.union(cd);
                    }
                    ClassExpr::DataHasValue(p, l) => {
                        exist.entry(p).or_default().push((1, DataRange::OneOf(vec![l.clone()])));
                        deps = deps.union(cd);
                    }
                    ClassExpr::DataMin(n, p, d) => {
                        exist.entry(p).or_default().push((*n, d.clone()));
                        deps = deps.union(cd);
                    }
                    ClassExpr::DataMax(n, p, d) => {
                        maxes.entry(p).or_default().push((*n, d));
                        deps = deps.union(cd);
                    }
                    _ => {}
                }
            }
            for (p, l, d) in &node.values {
                vals.entry(p).or_default().push(l.clone());
                deps = deps.union(d);
            }
            let relevant: BTreeSet<&Iri> = exist.keys().chain(maxes.keys()).chain(vals.keys()).copied().collect();
            if relevant.is_empty() {
                continue;
            }
            let supers_of = |q: &Iri| self.tbox.data_super.get(q).cloned().unwrap_or_else(|| BTreeSet::from([q.clone()]));
            // every property a value of which can count for a constrained one, grouped into hierarchy components
            let mut universe: BTreeSet<Iri> = relevant.iter().map(|p| (*p).clone()).collect();
            for q in self.tbox.data_super.keys() {
                if supers_of(q).iter().any(|s| relevant.contains(s)) {
                    universe.insert(q.clone());
                }
            }
            let members: Vec<Iri> = universe.into_iter().collect();
            let mut comp: Vec<usize> = (0..members.len()).collect();
            for i in 0..members.len() {
                for j in 0..members.len() {
                    if supers_of(&members[i]).contains(&members[j]) {
                        let (a, b) = (comp[i].min(comp[j]), comp[i].max(comp[j]));
                        for c in comp.iter_mut() {
                            if *c == b {
                                *c = a;
                            }
                        }
                    }
                }
            }
            // properties that are disjoint from one another share a group, since a value cannot be given to both
            let disjoint_pair = |i: usize, j: usize| {
                let (si, sj) = (supers_of(&members[i]), supers_of(&members[j]));
                self.tbox.data_disjoint.iter().any(|(a, b)| (si.contains(a) && sj.contains(b)) || (si.contains(b) && sj.contains(a)))
            };
            for i in 0..members.len() {
                for j in i..members.len() {
                    if disjoint_pair(i, j) {
                        let (a, b) = (comp[i].min(comp[j]), comp[i].max(comp[j]));
                        for c in comp.iter_mut() {
                            if *c == b {
                                *c = a;
                            }
                        }
                    }
                }
            }
            let roots: BTreeSet<usize> = comp.iter().copied().collect();
            for root in roots {
                let group: Vec<&Iri> = members.iter().enumerate().filter(|(i, _)| comp[*i] == root).map(|(_, m)| m).collect();
                if !group.iter().any(|p| relevant.contains(*p)) {
                    continue;
                }
                let empty: Vec<Literal> = Vec::new();
                let all_ranges: Vec<Vec<&DataRange>> = group.iter().map(|p| supers_of(p).iter().flat_map(|q| alls.get(q).into_iter().flatten().copied()).collect()).collect();
                let ex: Vec<Vec<(u32, &DataRange)>> = group.iter().map(|p| exist.get(*p).into_iter().flatten().map(|(n, d)| (*n, d)).collect()).collect();
                let mx: Vec<Vec<(u32, &DataRange)>> = group.iter().map(|p| maxes.get(*p).cloned().unwrap_or_default()).collect();
                let vs: Vec<&Vec<Literal>> = group.iter().map(|p| vals.get(*p).unwrap_or(&empty)).collect();
                let sup: Vec<u64> = group.iter().map(|p| {
                    let s = supers_of(p);
                    group.iter().enumerate().fold(0u64, |m, (j, q)| if s.contains(*q) { m | 1 << j } else { m })
                }).collect();
                let mut disj: Vec<u64> = Vec::new();
                for i in 0..group.len() {
                    for j in i..group.len() {
                        if disjoint_pair(members.iter().position(|m| m == group[i]).unwrap(), members.iter().position(|m| m == group[j]).unwrap()) {
                            disj.push(1 << i | 1 << j);
                        }
                    }
                }
                let key = format!("{vs:?}|{all_ranges:?}|{ex:?}|{mx:?}|{sup:?}|{disj:?}");
                let cached = self.data_cache.borrow().get(&key).copied();
                let ok = match cached {
                    Some(v) => v,
                    None => {
                        let props: Vec<crate::data::Prop> = (0..group.len()).map(|i| crate::data::Prop { vals: vs[i], alls: &all_ranges[i], exist: &ex[i], maxes: &mx[i] }).collect();
                        let v = crate::data::feasible_group(&props, &sup, &disj).map_err(|e| match e {
                            crate::data::DataError::Unsupported(m) => ReasonError::Unsupported(m),
                            crate::data::DataError::Budget => ReasonError::Budget("data"),
                        })?;
                        self.data_cache.borrow_mut().insert(key, v);
                        v
                    }
                };
                if !ok {
                    return Ok(Some(deps));
                }
            }
        }
        Ok(None)
    }

    /// A node that needs at least `n` neighbours via `r` and at most `m < n` via a super-role of `r` (same filler, or `m` holds
    /// for all) is contradictory; finding that now saves generating the `n` successors first.
    fn numeric_clash(&self, st: &mut State, x: NodeId) {
        if st.clash.is_some() {
            return;
        }
        let label = &st.nodes[x].label;
        let mins: Vec<(&u32, &Role, &ClassExpr, &Deps)> = label.iter().filter_map(|(c, d)| if let ClassExpr::Min(n, r, f) = c { Some((n, r, &**f, d)) } else { None }).collect();
        if mins.is_empty() {
            return;
        }
        for (c, md) in label {
            if let ClassExpr::Max(m, s, g) = c {
                for (n, r, f, nd) in &mins {
                    if **n > *m && self.tbox.hier.is_sub(r, s) && (**g == ClassExpr::Thing || **g == **f) {
                        st.clash = Some(md.union(nd));
                        return;
                    }
                }
            }
        }
    }

    /// One scheduling decision after the deterministic rules have settled. Only nodes that changed since they were last
    /// found quiet are looked at; rule classes keep their priority (choices and merges, then disjunctions, then generation).
    fn step(&self, st: &mut State) -> Step {
        *self.site.borrow_mut() = None;
        let blocked = match self.deterministic(st) {
            Ok(b) => b,
            Err(d) => return Step::Clash(d),
        };
        let mut live: Vec<NodeId> = st.pending.keys().copied().filter(|&i| st.nodes[i].alive && !blocked[i]).collect();
        st.pending.retain(|&i, _| st.nodes[i].alive && !blocked[i]);
        // oldest change first, so that `.rev()` below (and the generation order) start from what changed last
        if self.recent.get() {
            live.sort_by_cached_key(|i| st.pending.get(i).copied().unwrap_or(0));
        } else {
            // by role first (the rule loop below looks at the end of this list first): other individuals, then nominals, then
            // the node under test and what comes from it
            live.sort_by_cached_key(|&i| (if i >= st.abox_len { 2u8 } else { u8::from(st.nominal_nodes.contains(&i)) }, i));
        }
        // choose and ≤ rules
        for &x in &live {
            if !st.rules_le.contains(&x) {
                continue;
            }
            for (c, cd) in &st.nodes[x].label {
                if let ClassExpr::Max(n, r, d) = c {
                    let nb = self.neighbours(st, x, r);
                    if **d != ClassExpr::Thing {
                        let nd = neg(d);
                        for (y, ed) in &nb {
                            if !st.has(*y, d) && !st.has(*y, &nd) {
                                let premise = cd.union(ed);
                                return Step::Branch(vec![Choice::Add(*y, (**d).clone(), premise.clone()), Choice::Add(*y, nd, premise)]);
                            }
                        }
                    }
                    let inside: Vec<(NodeId, Deps)> = nb.into_iter().filter(|(y, _)| **d == ClassExpr::Thing || st.has(*y, d)).collect();
                    // NN-rule: a tree node pointing at a root that has `≤ n S.C` is promoted to a root itself
                    if st.nodes[x].root {
                        let promote: Vec<NodeId> = inside.iter().map(|(y, _)| *y).filter(|&y| !st.nodes[y].root && st.nodes[y].parent != Some(x)).collect();
                        if !promote.is_empty() {
                            for y in promote {
                                st.promote(y);
                            }
                            return Step::Applied;
                        }
                    }
                    if inside.len() > *n as usize {
                        let subset = &inside[..*n as usize + 1];
                        let mut premise = cd.clone();
                        for (y, ed) in subset {
                            premise = premise.union(ed);
                            if **d != ClassExpr::Thing {
                                premise = premise.union(&st.dep(*y, d));
                            }
                        }
                        let mut alts = Vec::new();
                        let mut clash = premise.clone();
                        for i in 0..subset.len() {
                            for j in (i + 1)..subset.len() {
                                match st.different(subset[i].0, subset[j].0) {
                                    None => alts.push(Choice::Merge(subset[i].0, subset[j].0, premise.clone())),
                                    Some(nd) => clash = clash.union(nd),
                                }
                            }
                        }
                        if alts.is_empty() {
                            return Step::Clash(clash);
                        }
                        *self.site.borrow_mut() = Some((x, c.clone()));
                        return Step::Branch(alts);
                    }
                }
            }
            st.rules_le.remove(&x);
        }
        // ⊔ rule. The newest nodes are decided first: the node a test is about (and what is generated from it) is settled before
        // the choices about all the older individuals, so a clash that only involves it does not drag those choices into the
        // search.
        for &x in live.iter().rev() {
            if !st.rules_or.contains(&x) {
                continue;
            }
            for (c, cd) in &st.nodes[x].label {
                if let ClassExpr::Or(v) = c {
                    if !v.contains(&ClassExpr::Thing) && !v.iter().any(|d| st.has(x, d)) {
                        // try the cheap, non-generating alternatives first: a wrong guess that creates nodes costs a whole subtree
                        let mut order: Vec<&ClassExpr> = v.iter().collect();
                        {
                            let failed = self.failures.borrow();
                            let pref = self.phase.borrow().get(&(x, c.clone())).cloned();
                            order.sort_by_key(|d| (u8::from(pref.as_ref() != Some(*d)), failed.get(*d).copied().unwrap_or(0).min(64), branch_cost(d)));
                        }
                        let mut alts = Vec::new();
                        for (i, d) in order.iter().enumerate() {
                            // semantic branching: the i-th alternative also excludes the earlier ones, so the branches do not overlap
                            let mut all = vec![(*d).clone()];
                            all.extend(order[..i].iter().map(|e| neg(e)));
                            alts.push(if all.len() == 1 { Choice::Add(x, all.pop().unwrap(), cd.clone()) } else { Choice::AddAll(x, all, cd.clone()) });
                        }
                        return Step::Branch(alts);
                    }
                }
            }
            st.rules_or.remove(&x);
        }
        // generating rules; a node with nothing to generate has now passed every rule class and is quiet. One round generates
        // for every waiting node (one requirement each), so scanning the waiting nodes is paid once per round, not per node.
        let mut generated = false;
        let round_start = st.tick;
        // blocking is worked out afresh before every round of generation, so a node that is now blocked does not spawn a whole
        // new level (the lazy schedule alone would let a binary tree double in size between two looks)
        let before_fresh = st.was_blocked.clone();
        let fresh = self.refresh_blocked(st, true);
        if fresh.iter().enumerate().any(|(i, now)| !now && before_fresh.get(i).copied().unwrap_or(false)) {
            // a node was freed (and queued) by this look: it has rules to run before anything is generated or declared complete
            return Step::Applied;
        }
        for &x in &live {
            if fresh.get(x).copied().unwrap_or(false) {
                continue;
            }
            // existentials over a nominal come first and become an edge to the individual's node at once (no helper node to merge
            // later): a root that has `≤ n S.C` can then see this node before the node expands further, which is what keeps a chain
            // of nodes that all point at the root finite
            let mut to_nominal: Option<(Role, NodeId, Deps)> = None;
            for (c, cd) in &st.nodes[x].label {
                if let ClassExpr::Some(r, d) = c {
                    if let ClassExpr::OneOf(v) = &**d {
                        if v.len() == 1 {
                            if let Some(&h0) = st.home.get(&v[0]) {
                                let (h, hd) = st.resolve_deps(h0);
                                if !self.neighbours(st, x, r).iter().any(|(y, _)| *y == h) {
                                    to_nominal = Some((r.clone(), h, cd.union(&hd)));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            if let Some((r, h, d)) = to_nominal {
                st.add_edge(x, &r, h, &d);
                generated = true;
                continue;
            }
            let mut todo: Option<(ClassExpr, Deps)> = None;
            for (c, cd) in &st.nodes[x].label {
                match c {
                    ClassExpr::Some(r, d) => {
                        if !self.neighbours(st, x, r).iter().any(|(y, _)| **d == ClassExpr::Thing || st.has(*y, d)) {
                            todo = Some((c.clone(), cd.clone()));
                            break;
                        }
                    }
                    ClassExpr::Min(n, r, d) => {
                        let cands: Vec<NodeId> = self.neighbours(st, x, r).into_iter().map(|(y, _)| y).filter(|&y| **d == ClassExpr::Thing || st.has(y, d)).collect();
                        if !has_distinct_set(st, &cands, *n as usize, &mut Vec::new()) {
                            todo = Some((c.clone(), cd.clone()));
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match todo {
                Some((ClassExpr::Some(r, d), cd)) => {
                    let y = st.new_node(Some(x), false, self.initial_label([((*d).clone(), cd.clone())]));
                    st.add_edge(x, &r, y, &cd);
                    generated = true;
                }
                Some((ClassExpr::Min(n, r, d), cd)) => {
                    // The budget is checked between steps, and this one step makes `n` nodes and n*(n-1)/2 inequalities (a count of
                    // 60000 is 1.8 billion of them): refuse before building them, not after.
                    let n_wide = n as usize;
                    if st.nodes.len().saturating_add(n_wide) > self.max_nodes {
                        return Step::Fail(ReasonError::Budget("nodes"));
                    }
                    if st.neq.len().saturating_add(n_wide.saturating_mul(n_wide) / 2) > self.max_cells {
                        return Step::Fail(ReasonError::Budget("memory"));
                    }
                    let mut fresh = Vec::new();
                    for _ in 0..n {
                        let y = st.new_node(Some(x), false, self.initial_label([((*d).clone(), cd.clone())]));
                        st.add_edge(x, &r, y, &cd);
                        fresh.push(y);
                    }
                    for i in 0..fresh.len() {
                        for j in (i + 1)..fresh.len() {
                            st.neq.insert(pair(fresh[i], fresh[j]), cd.clone());
                        }
                    }
                    generated = true;
                }
                _ => {
                    // quiet - unless something in this very round queued it again (a neighbour that just got an edge to it)
                    if st.pending.get(&x).is_some_and(|&t| t <= round_start) {
                        st.pending.remove(&x);
                    }
                }
            }
        }
        if generated {
            return Step::Applied;
        }
        // the flags used so far may be stale: work them out in full, and carry on if that frees a node
        let before = st.was_blocked.clone();
        let blocked = self.refresh_blocked(st, true);
        if blocked.iter().enumerate().any(|(i, now)| !now && before.get(i).copied().unwrap_or(false)) {
            return Step::Applied;
        }
        if self.has_data {
            match self.data_check(st, &blocked) {
                Ok(None) => {}
                Ok(Some(d)) => return Step::Clash(d),
                Err(e) => return Step::Fail(e),
            }
        }
        Step::Complete
    }

    /// Apply one alternative of the branch point at `level`. `Err` carries the clash dependencies.
    fn apply(&self, st: &mut State, c: &Choice, level: u32) -> Result<(), Deps> {
        st.max_level = st.max_level.max(level);
        match c {
            Choice::Add(x, d, premise) => {
                st.add_concept(*x, d.clone(), premise.with(level));
                Ok(())
            }
            Choice::AddAll(x, ds, premise) => {
                for d in ds {
                    st.add_concept(*x, d.clone(), premise.with(level));
                }
                Ok(())
            }
            Choice::Merge(a, b, premise) => {
                let deps = premise.with(level);
                // direction: a root absorbs a tree node; a parent absorbs its child; otherwise the younger goes into the older
                let (from, into) = if st.nodes[*a].root && !st.nodes[*b].root {
                    (*b, *a)
                } else if (st.nodes[*b].root && !st.nodes[*a].root) || st.nodes[*a].parent == Some(*b) {
                    (*a, *b)
                } else if st.nodes[*b].parent == Some(*a) {
                    (*b, *a)
                } else if a > b {
                    (*a, *b)
                } else {
                    (*b, *a)
                };
                st.merge(from, into, &deps)
            }
        }
    }

    /// Expand to completion. `Ok(Some(state))` is a clash-free complete graph (a model); `Ok(None)` means no model exists.
    /// Clashes backjump straight to the most recent branch point they depend on, skipping the choices in between.
    fn run(&self, init: State) -> Res<Option<State>> {
        self.run_capped(init, self.max_steps)
    }

    /// `run` with its own step limit (for tests whose failure to finish is not an answer).
    fn run_capped(&self, init: State, max_steps: usize) -> Res<Option<State>> {
        Ok(self.search(init, Vec::new(), 0, max_steps, false, &|_| {})?.0)
    }

    /// The search itself. `stack` holds the choice points of a search that was already under way (empty for a fresh one), and
    /// `assume` adds the extra facts of this question to a graph; it is applied to `init` by the caller and again to every saved
    /// graph the search backtracks to. Resuming is exact: an alternative that failed without the extra facts fails with them too,
    /// and every alternative that was never tried is still on the stack. Returns the complete graph (or none) and the stack.
    ///
    /// `offset` > 0 means that facts in `init` depend on branch levels `1..=offset` which are *not* reopened (a completed model
    /// that is only being extended): a clash that depends on one of them ends the search with "no model", which then only says
    /// that this particular completion does not fit.
    fn search(&self, init: State, mut stack: Vec<Arc<Entry>>, offset: u32, max_steps: usize, recent: bool, assume: &dyn Fn(&mut State)) -> Res<(Option<State>, Vec<Arc<Entry>>)> {
        if let Some(why) = &self.tbox.chain_error {
            return Err(ReasonError::Unsupported(why.clone()));
        }
        self.recent.set(recent);
        let mut st = init;
        let mut steps = 0usize;
        let (mut t_step, mut t_snap, mut t_back) = (std::time::Duration::ZERO, std::time::Duration::ZERO, std::time::Duration::ZERO);
        let timing = self.trace.is_some();
        // the memory check walks every held node, so it runs on a clock and backs off when it is itself slow
        let (mut last_check, mut gap) = (std::time::Instant::now(), std::time::Duration::ZERO);
        loop {
            steps += 1;
            if last_check.elapsed() >= gap {
                let t = std::time::Instant::now();
                let cells = Self::live_cells(&st, &stack);
                if self.trace.is_some() {
                    eprintln!("memory check at step {steps}: {cells} cells, {} nodes, {} levels, {} alternatives held, {} inequalities in saved states, {:?} since the last check", st.nodes.len(), stack.len(), stack.iter().map(|e| e.alts.len()).sum::<usize>(), stack.iter().map(|e| e.snap.neq.len()).sum::<usize>(), last_check.elapsed());
                }
                if cells > self.max_cells {
                    return Err(ReasonError::Budget("memory"));
                }
                gap = (t.elapsed() * 4).max(std::time::Duration::from_millis(250));
                last_check = std::time::Instant::now();
            }
            self.last_run.set((steps, st.nodes.len(), stack.len()));
            if self.trace.is_some() && steps.is_multiple_of(50_000) {
                eprintln!("search still going at step {steps}, {} nodes, {} levels, {} roots", st.nodes.len(), stack.len(), st.nodes.iter().filter(|n| n.root && n.alive).count());
            }
            if steps > max_steps {
                return Err(ReasonError::Budget("steps"));
            }
            if st.nodes.len() > self.max_nodes {
                return Err(ReasonError::Budget("nodes"));
            }
            let mut clash: Option<Deps> = None;
            let t0 = std::time::Instant::now();
            let outcome = self.step(&mut st);
            if timing {
                t_step += t0.elapsed();
            }
            if self.trace.is_some_and(|n| steps < n) {
                let roots: Vec<usize> = st.nodes.iter().enumerate().filter(|(_, n)| n.root && n.alive).map(|(i, _)| i).collect();
                let blocked: Vec<usize> = st.was_blocked.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect();
                eprintln!("  [step {steps}] {} nodes {} roots {roots:?} blocked {blocked:?}", match &outcome { Step::Applied => "applied", Step::Complete => "complete", Step::Clash(_) => "clash", Step::Branch(_) => "branch", Step::Fail(_) => "fail" }, st.nodes.len());
            }
            match outcome {
                Step::Fail(e) => return Err(e),
                Step::Complete => {
                    if self.learn.get() {
                        let mut phase = self.phase.borrow_mut();
                        for e in &stack {
                            if let (Some(site), Some(cur)) = (&e.site, &e.cur) {
                                phase.insert(site.clone(), cur.clone());
                            }
                        }
                    }
                    if timing && steps > 500 {
                        eprintln!("  search done: {steps} steps; step() {t_step:?}, snapshots {t_snap:?}, backtracking {t_back:?}");
                    }
                    return Ok((Some(st), stack));
                }
                Step::Applied => {}
                Step::Clash(d) => clash = Some(d),
                Step::Branch(mut alts) => {
                    let level = offset + stack.len() as u32 + 1;
                    if self.trace.is_some_and(|n| steps < n) {
                        let d = |c: &Choice| match c {
                            Choice::AddAll(x, e, _) => format!("n{x}+all{e:?}"),
                            Choice::Add(x, e, _) => format!("n{x}+{}", format!("{e:?}").replace("Iri(\"", "").replace("\")", "").replace("http://www.informatik.uni-ulm.de/ki/Liebig/reasoner-eval/6.owl#", "")),
                            Choice::Merge(a, b, _) => format!("merge {a} {b}"),
                        };
                        eprintln!("step {steps} level {level} nodes {}: {}", st.nodes.len(), alts.iter().map(d).collect::<Vec<_>>().join("  |  "));
                    }
                    alts.reverse();
                    let t1 = std::time::Instant::now();
                    let snap = st.clone();
                    if timing {
                        t_snap += t1.elapsed();
                    }
                    let first = alts.pop().expect("a branch has alternatives");
                    let cur = choice_key(&first);
                    let site = self.site.borrow_mut().take();
                    stack.push(Arc::new(Entry { snap, alts, acc: Deps::new(), cur, site }));
                    st.nondet = true;
                    if let Err(d) = self.apply(&mut st, &first, level) {
                        clash = Some(d);
                    }
                }
            }
            let Some(mut d) = clash else { continue };
            if self.trace.is_some_and(|n| steps < n) {
                eprintln!("  clash at step {steps}, depends on levels {:?}", d.items());
            }
            let t2 = std::time::Instant::now();
            loop {
                let Some(l) = d.max() else {
                    if timing {
                        eprintln!("  search ended: {steps} steps; step() {t_step:?}, snapshots {t_snap:?}, backtracking {t_back:?}");
                    }
                    return Ok((None, stack));
                };
                if l <= offset {
                    return Ok((None, stack));
                }
                stack.truncate((l - offset) as usize);
                let e = Arc::make_mut(stack.last_mut().expect("a dependency names a live branch point"));
                e.acc = e.acc.union(&d.without(l));
                if let Some(k) = e.cur.take() {
                    *self.failures.borrow_mut().entry(k).or_insert(0) += 1;
                }
                match e.alts.pop() {
                    Some(alt) => {
                        e.cur = choice_key(&alt);
                        let mut next = e.snap.clone();
                        next.nondet = true;
                        assume(&mut next);
                        match self.apply(&mut next, &alt, l) {
                            Ok(()) => {
                                st = next;
                                if timing {
                                    t_back += t2.elapsed();
                                }
                                break;
                            }
                            Err(d2) => d = d2,
                        }
                    }
                    None => {
                        d = std::mem::take(&mut e.acc);
                        stack.pop();
                    }
                }
            }
        }
    }

    // ---- ABox
    fn abox_state(&self, extra: Option<(&Iri, ClassExpr)>) -> Option<(State, BTreeMap<Iri, NodeId>)> {
        self.build_state(None, extra)
    }

    /// The starting graph: a node per individual with the asserted facts about it. `only` restricts it to some individuals and
    /// to the assertions that mention only them (a *relaxation* of the ontology: whatever has no model here has none with all
    /// the facts either). `None` = an assertion contradicts another, so there is no model at all.
    fn build_state(&self, only: Option<&BTreeSet<Iri>>, extra: Option<(&Iri, ClassExpr)>) -> Option<(State, BTreeMap<Iri, NodeId>)> {
        let none = Deps::new();
        let mut st = State::default();
        let mut ids: BTreeMap<Iri, NodeId> = BTreeMap::new();
        for i in self.onto.individual_names() {
            if only.is_some_and(|o| !o.contains(&i)) {
                continue;
            }
            // with nominals, an individual's node is the nominal `{i}`, so axioms stated about nominals apply to it
            let own = if self.nominals { vec![(ClassExpr::OneOf(vec![i.clone()]), Deps::new())] } else { Vec::new() };
            let id = st.new_node(None, true, self.initial_label(own));
            ids.insert(i, id);
        }
        for a in &self.onto.axioms {
            match a {
                Axiom::ClassAssertion(c, i) => {
                    if let Some(&x) = ids.get(i) {
                        st.add_concept(x, nnf(c), none.clone());
                    }
                }
                Axiom::ObjectPropertyAssertion(r, a, b) => {
                    if let (Some(&x), Some(&y)) = (ids.get(a), ids.get(b)) {
                        st.add_edge(x, r, y, &none);
                    }
                }
                Axiom::DataPropertyAssertion(p, a, l) => {
                    if let Some(&x) = ids.get(a) {
                        st.add_value(x, p.clone(), l.clone(), none.clone());
                    }
                }
                Axiom::DifferentIndividuals(v) => {
                    for i in 0..v.len() {
                        for j in (i + 1)..v.len() {
                            let (Some(&x), Some(&y)) = (ids.get(&v[i]), ids.get(&v[j])) else { continue };
                            if x == y {
                                return None;
                            }
                            st.neq.insert(pair(x, y), none.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        for a in &self.onto.axioms {
            if let Axiom::SameIndividual(v) = a {
                for w in v.windows(2) {
                    let (Some(&x), Some(&y)) = (ids.get(&w[0]), ids.get(&w[1])) else { continue };
                    if x != y {
                        if st.merge(y, x, &none).is_err() {
                            return None;
                        }
                        for id in ids.values_mut() {
                            if *id == y {
                                *id = x;
                            }
                        }
                    }
                }
            }
        }
        st.home = Arc::new(ids.clone());
        let noms = self.onto.nominal_names();
        st.nominal_nodes = Arc::new(ids.iter().filter(|(i, _)| noms.contains(*i)).map(|(_, &n)| n).collect());
        st.abox_len = st.nodes.len();
        // a key applies to the named instances of its class, so every named individual must decide whether it is one
        for (c, _, _) in &self.tbox.keys {
            let decide = ClassExpr::Or(vec![c.clone(), neg(c)]);
            for &x in ids.values() {
                let node = st.resolve(x);
                st.add_concept(node, decide.clone(), none.clone());
            }
        }
        for (i, c) in &self.tbox.nominal_facts {
            if let Some(&x) = ids.get(i) {
                let node = st.resolve(x);
                st.add_concept(node, c.clone(), none.clone());
            }
        }
        if let Some((i, c)) = extra {
            let node = st.resolve(ids[i]);
            st.add_concept(node, c, none);
        }
        Some((st, ids))
    }

    // ---- public inferences
    /// The completed graph of the whole ontology, or `None` if it is inconsistent. Computed once.
    fn base_model(&self) -> Res<&Option<AboxModel>> {
        self.base
            .get_or_init(|| {
                let Some((st, ids)) = self.abox_state(None) else { return Ok(None) };
                if st.nodes.is_empty() {
                    return Ok(self.is_satisfiable_uncached(&ClassExpr::Thing)?.then(|| (State::default(), BTreeMap::new(), Vec::new())));
                }
                self.learn.set(true);
                let found = self.search(st, Vec::new(), 0, self.max_steps, true, &|_| {});
                self.learn.set(false);
                let (m, stack) = found?;
                Ok(m.map(|m| (m, ids, stack)))
            })
            .as_ref()
            .map_err(|e| e.clone())
    }

    /// Does the label of a completed model of a class refute that the class is below the lazily defined class `b`? It does when
    /// the definition of `b` has a conjunct that is a class (or the negation of one) that is not lazy, so that the label settles
    /// it, and the label says the opposite.
    fn model_fails_definition(&self, label: &BTreeMap<ClassExpr, bool>, b: &Iri) -> bool {
        fn conjuncts<'a>(c: &'a ClassExpr, out: &mut Vec<&'a ClassExpr>) {
            match c {
                ClassExpr::And(v) => v.iter().for_each(|x| conjuncts(x, out)),
                other => out.push(other),
            }
        }
        let Some(defs) = self.tbox.unfold.get(b) else { return false };
        let mut parts = Vec::new();
        for d in defs {
            conjuncts(d, &mut parts);
        }
        parts.into_iter().any(|p| match p {
            ClassExpr::Class(x) if !self.tbox.lazy.contains(x) => !label.contains_key(p),
            ClassExpr::Not(inner) => matches!(&**inner, ClassExpr::Class(x) if !self.tbox.lazy.contains(x) && label.contains_key(&**inner)),
            _ => false,
        })
    }

    /// Is the whole ontology (TBox and ABox) consistent?
    pub fn is_consistent(&self) -> Res<bool> {
        Ok(self.base_model()?.is_some())
    }

    /// A model of `c` with the TBox, as the completed graph and the node `c` was tested at. With nominals the individuals are
    /// part of every model, so the test starts from their nodes: a new node for `c` is added to the completed model of the
    /// individuals and the search resumes from there, backtracking into the choices made about the individuals only as far as a
    /// clash requires.
    fn concept_model(&self, c: &ClassExpr) -> Res<Option<(State, NodeId)>> {
        let t0 = std::time::Instant::now();
        self.failures.borrow_mut().clear();
        let r = self.concept_model_inner(c);
        if self.trace.is_some() && t0.elapsed().as_millis() > 300 {
            eprintln!("concept test took {:?}: {}", t0.elapsed(), format!("{c:?}").replace("Iri(\"http://www.w3.org/TR/2003/PR-owl-guide-20031209/wine#", "").chars().take(160).collect::<String>());
        }
        r
    }

    fn say(&self, what: &str, cap: usize) {
        if self.trace.is_some() {
            eprintln!("  settled by {what} at limit {cap}");
        }
    }

    fn concept_model_inner(&self, c: &ClassExpr) -> Res<Option<(State, NodeId)>> {
        if self.nominals {
            let Some((base, _, stack)) = self.base_model()? else { return Ok(None) };
            let label = self.initial_label([(nnf(c), Deps::new())]);
            let at = std::cell::Cell::new(0usize);
            let assume = |st: &mut State| at.set(st.new_node(None, true, label.clone()));
            let nominals = self.onto.nominal_names();
            // The strategies below are all exact (or only ever say "satisfiable" when they found a model), but each is much faster
            // than the others on some questions. Try each with a small step limit, then quadruple the limit and go round again, so
            // the cost stays within a small factor of the best strategy for the question at hand. The strategy that worked last
            // goes first: neighbouring questions are alike.
            // Facts about the individuals only add constraints, so if `c` has no model even with just the nominals (and the facts
            // that mention only them), it has none with all the facts either: "unsatisfiable". If it has one, what that model
            // says about each nominal is a good guess for the whole ontology (see strategy 4).
            // The quick fit is tried before anything else: when it finds a model the relaxation below is not needed at all, and for
            // most satisfiable classes it does.
            let mut fit_cap = 1_000usize;
            {
                let mut init = base.clone();
                assume(&mut init);
                match self.search(init, Vec::new(), stack.len() as u32, fit_cap, false, &assume) {
                    Ok((Some(m), _)) => {
                        self.say("fit", fit_cap);
                        return Ok(Some((m, at.get())));
                    }
                    Ok((None, _)) | Err(ReasonError::Budget(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            let nominal_facts: Vec<(Iri, ClassExpr)> = match self.build_state(Some(&nominals), None) {
                None => return Ok(None),
                Some((mut st, ids)) => {
                    st.new_node(None, true, self.initial_label([(nnf(c), Deps::new())]));
                    match self.run_capped(st, 20_000) {
                        Ok(None) => {
                            self.say("relaxation: unsatisfiable", 0);
                            return Ok(None);
                        }
                        Ok(Some(m)) => ids
                            .iter()
                            .flat_map(|(i, &n)| {
                                let (at, _) = m.resolve_deps(n);
                                m.nodes[at].label.keys().map(move |k| (i.clone(), k.clone()))
                            })
                            .collect(),
                        Err(ReasonError::Budget(_)) => Vec::new(),
                        Err(e) => return Err(e),
                    }
                }
            };
            // the quick fit needs little; every other strategy first has to replay the choices about all the individuals (a step
            // per choice), so a smaller limit than a few thousand steps would only waste it
            let first_cap = self.start_cap.get().max(4_000);
            let mut cap = first_cap;
            loop {
                let first = self.hint.get();
                for k in (0..5).map(|i| (first + i) % 5) {
                    // (resuming costs several times more per step than the others, so it gets a quarter of the limit)
                    let cap = match k {
                        0 => fit_cap,
                        3 | 4 => (cap / 4).max(1_000),
                        _ => cap,
                    };
                    let t_k = std::time::Instant::now();
                    let result = match k {
                        // a new node for `c` in a copy of the completed model, whose choices stay as they are (only finds models)
                        0 => {
                            let mut init = base.clone();
                            assume(&mut init);
                            match self.search(init, Vec::new(), stack.len() as u32, cap, false, &assume) {
                                Ok((Some(m), _)) => Ok(Some(Some((m, at.get())))),
                                Ok((None, _)) => Ok(None),
                                Err(e) => Err(e),
                            }
                        }
                        // resume the saved search of the whole ontology with a new node for `c`, looking at the oldest nodes first
                        // (3) or at the most recently changed first (4); both exact
                        3 | 4 => {
                            let mut init = base.clone();
                            assume(&mut init);
                            self.search(init, stack.clone(), 0, cap, k == 4, &assume).map(|(m, _)| Some(m.map(|m| (m, at.get()))))
                        }
                        // the full search with what the relaxed model says about the nominals given as facts: only finds models
                        2 => match self.abox_state(None) {
                            None => Ok(None),
                            Some((mut st, ids)) => {
                                for (i, k) in &nominal_facts {
                                    let node = st.resolve(ids[i]);
                                    st.add_concept(node, k.clone(), Deps::new());
                                }
                                let node = st.new_node(None, true, self.initial_label([(nnf(c), Deps::new())]));
                                match self.search(st, Vec::new(), 0, cap, false, &|_| {}) {
                                    Ok((Some(m), _)) => Ok(Some(Some((m, node)))),
                                    Ok((None, _)) => Ok(None),
                                    Err(e) => Err(e),
                                }
                            }
                        },
                        // the full search from the start, where the choices about `c`'s node come first (exact)
                        _ => match self.abox_state(None) {
                            None => Ok(Some(None)),
                            Some((mut st, _)) => {
                                let node = st.new_node(None, true, self.initial_label([(nnf(c), Deps::new())]));
                                self.search(st, Vec::new(), 0, cap, false, &|_| {}).map(|(m, _)| Some(m.map(|m| (m, node))))
                            }
                        },
                    };
                    if self.trace.is_some() && t_k.elapsed().as_millis() > 200 {
                        eprintln!("  strategy {k} at limit {cap}: {:?} ({})", t_k.elapsed(), if matches!(result, Ok(Some(_))) { "settled" } else { "no answer" });
                    }
                    match result {
                        Ok(Some(answer)) => {
                            self.hint.set(k);
                            // settled in the first round: the questions are getting easier, so the starting limit decays
                            self.start_cap.set(if cap == first_cap { (first_cap * 3 / 4).max(4_000) } else { cap });
                            self.say(["fit", "fresh search", "guided by the relaxed model", "resume (roles first)", "resume (recent first)"][k], cap);
                            return Ok(answer);
                        }
                        Ok(None) | Err(ReasonError::Budget(_)) => {}
                        Err(e) => return Err(e),
                    }
                }
                if cap >= self.max_steps && fit_cap >= self.max_steps {
                    return Err(ReasonError::Budget("steps"));
                }
                fit_cap = (fit_cap * 4).min(self.max_steps);
                cap = (cap * 4).min(self.max_steps);
            }
        }
        let mut st = State::default();
        let node = st.new_node(None, true, self.initial_label([(nnf(c), Deps::new())]));
        Ok(self.run(st)?.map(|m| (m, node)))
    }

    fn is_satisfiable_uncached(&self, c: &ClassExpr) -> Res<bool> {
        Ok(self.concept_model(c)?.is_some())
    }

    /// Satisfiability of `c` with respect to the TBox.
    pub fn is_satisfiable(&self, c: &ClassExpr) -> Res<bool> {
        self.is_satisfiable_uncached(c)
    }

    /// `sub ⊑ sup`?
    pub fn subsumes(&self, sup: &ClassExpr, sub: &ClassExpr) -> Res<bool> {
        let t = std::time::Instant::now();
        let r = !self.is_satisfiable(&ClassExpr::And(vec![sub.clone(), neg(sup)]))?;
        if self.trace.is_some() && t.elapsed().as_millis() > 1000 {
            eprintln!("slow subsumes ({:?}): {sub:?} <= {sup:?} = {r}", t.elapsed());
        }
        Ok(r)
    }

    /// Is `ind` an instance of `c`? Errors with [`ReasonError::Inconsistent`] if the ontology has no model.
    pub fn instance_of(&self, ind: &Iri, c: &ClassExpr) -> Res<bool> {
        if self.trace.is_some() {
            eprintln!("instance_of {} {}", ind.local(), format!("{c:?}").replace("Iri(\"http://www.w3.org/TR/2003/PR-owl-guide-20031209/wine#", ""));
        }
        let Some((base, ids, stack)) = self.base_model()? else { return Err(ReasonError::Inconsistent) };
        if !self.onto.individual_names().contains(ind) {
            return Ok(false);
        }
        // relaxation: only `ind`, the nominals and the individuals asserted to be next to `ind`, with the facts that mention only
        // those. If "`ind` is not a `c`" has no model even there, it has none with all the facts, so `ind` is an instance.
        if self.nominals {
            let mut near = self.onto.nominal_names();
            near.insert(ind.clone());
            for a in &self.onto.axioms {
                if let Axiom::ObjectPropertyAssertion(_, x, y) = a {
                    if x == ind {
                        near.insert(y.clone());
                    } else if y == ind {
                        near.insert(x.clone());
                    }
                }
            }
            match self.build_state(Some(&near), Some((ind, neg(c)))) {
                None => return Ok(true),
                Some((st, _)) => match self.run_capped(st, 50_000) {
                    Ok(None) => return Ok(true),
                    Ok(Some(_)) | Err(ReasonError::Budget(_)) => {}
                    Err(e) => return Err(e),
                },
            }
        }
        // the exact test, by several strategies in turn with a growing step limit (see `concept_model_inner`): resume the search of
        // the whole ontology with "`ind` is not a `c`" added, looking at the most recently changed nodes first or at the oldest
        // first, or search from the start
        let (home, not_c) = (ids[ind], neg(c));
        let assume = |st: &mut State| {
            // (the node may have been merged into another by a choice: the fact then holds only under that choice)
            let (node, via) = st.resolve_deps(home);
            st.add_concept(node, not_c.clone(), via);
        };
        let mut cap = 1_000usize;
        loop {
            for recent in [true, false] {
                let mut init = base.clone();
                assume(&mut init);
                match self.search(init, stack.clone(), 0, cap, recent, &assume) {
                    Ok((m, _)) => return Ok(m.is_none()),
                    Err(ReasonError::Budget(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            if let Some((st, _)) = self.abox_state(Some((ind, neg(c)))) {
                match self.search(st, Vec::new(), 0, cap, false, &|_| {}) {
                    Ok((m, _)) => return Ok(m.is_none()),
                    Err(ReasonError::Budget(_)) => {}
                    Err(e) => return Err(e),
                }
            } else {
                return Ok(true);
            }
            if cap >= self.max_steps {
                return Err(ReasonError::Budget("steps"));
            }
            cap = (cap * 4).min(self.max_steps);
        }
    }

    /// Named classes equivalent to `owl:Thing` (they hold for everything, so they are reported separately).
    pub fn top_equivalents(&self) -> Res<BTreeSet<Iri>> {
        self.tops
            .get_or_init(|| {
                let mut out = BTreeSet::new();
                for c in &self.named {
                    if !self.is_satisfiable(&neg(&ClassExpr::Class(c.clone())))? {
                        out.insert(c.clone());
                    }
                }
                Ok(out)
            })
            .clone()
    }

    /// Named classes `ind` belongs to, except those equivalent to `owl:Thing` (see [`Self::top_equivalents`]).
    ///
    /// One model of the whole ontology answers most of it: a class that is not in `ind`'s label (and is decided at every
    /// node, i.e. not lazily defined) is false in that model, so `ind` is not an instance; a class in the label of a model
    /// built without any choice is entailed. Only the remaining cases need a refutation test.
    pub fn types(&self, ind: &Iri) -> Res<BTreeSet<Iri>> {
        if !self.is_consistent()? {
            return Err(ReasonError::Inconsistent);
        }
        let mut out = BTreeSet::new();
        if !self.onto.individual_names().contains(ind) {
            return Ok(out);
        }
        let top = self.top_equivalents()?;
        let Some((st, ids, _)) = self.base_model()? else { return Err(ReasonError::Inconsistent) };
        let (node, via) = st.resolve_deps(ids[ind]);
        let label = &st.nodes[node].label;
        for c in self.named.iter().filter(|c| !top.contains(*c)) {
            let target = ClassExpr::Class(c.clone());
            let entailed = if let Some(d) = label.get(&target) {
                d.union(&via).items().is_empty() || self.instance_of(ind, &target)?
            } else if self.tbox.lazy.contains(c) && !label.contains_key(&ClassExpr::Not(Box::new(target.clone()))) {
                self.instance_of(ind, &target)?
            } else {
                false
            };
            if entailed {
                out.insert(c.clone());
            }
        }
        Ok(out)
    }

    /// Classify every named class.
    /// What the axioms say outright: `A ⊑ B` for `SubClassOf(A B)`, for a named conjunct `B` of the right side, and for the named
    /// members and conjuncts of the other sides of an equivalence; closed under transitivity. Never wrong, usually incomplete.
    fn told_supers(&self) -> BTreeMap<Iri, BTreeSet<Iri>> {
        fn conjuncts(c: &ClassExpr, out: &mut Vec<Iri>) {
            match c {
                ClassExpr::Class(i) => out.push(i.clone()),
                ClassExpr::And(v) => v.iter().for_each(|x| conjuncts(x, out)),
                _ => {}
            }
        }
        let mut direct: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
        for ax in &self.onto.axioms {
            match ax {
                Axiom::SubClassOf(ClassExpr::Class(a), d) => {
                    let mut v = Vec::new();
                    conjuncts(d, &mut v);
                    direct.entry(a.clone()).or_default().extend(v);
                }
                Axiom::EquivalentClasses(m) => {
                    for x in m {
                        if let ClassExpr::Class(a) = x {
                            for y in m {
                                if y != x {
                                    let mut v = Vec::new();
                                    conjuncts(y, &mut v);
                                    direct.entry(a.clone()).or_default().extend(v);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let mut out: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
        for a in direct.keys() {
            let mut seen: BTreeSet<Iri> = BTreeSet::new();
            let mut todo: Vec<&Iri> = direct[a].iter().collect();
            while let Some(c) = todo.pop() {
                if seen.insert(c.clone()) {
                    todo.extend(direct.get(c).into_iter().flatten());
                }
            }
            seen.remove(a);
            out.insert(a.clone(), seen);
        }
        out
    }

    pub fn classify(&self) -> Res<Classification> {
        if !self.is_consistent()? {
            return Err(ReasonError::Inconsistent);
        }
        let names: Vec<Iri> = self.named.iter().cloned().collect();
        let mut unsat = BTreeSet::new();
        let mut models: BTreeMap<Iri, BTreeMap<ClassExpr, bool>> = BTreeMap::new();
        let mut states: BTreeMap<Iri, (State, NodeId)> = BTreeMap::new();
        for n in &names {
            match self.concept_model(&ClassExpr::Class(n.clone()))? {
                None => {
                    unsat.insert(n.clone());
                }
                Some((st, node)) => {
                    // each concept in the model's label, and whether it was derived without any choice (no branch dependency)
                    // (the node may have been merged into another one by a choice: what holds there holds for the tested concept only
                    // under that choice, so the merges on the way count as dependencies of every entry)
                    let (at, via) = st.resolve_deps(node);
                    models.insert(n.clone(), st.nodes[at].label.iter().map(|(k, d)| (k.clone(), d.union(&via).items().is_empty())).collect::<BTreeMap<_, _>>());
                    states.insert(n.clone(), (st, node));
                }
            }
        }
        let mut top = BTreeSet::new();
        for n in names.iter().filter(|n| !unsat.contains(*n)) {
            if !self.is_satisfiable(&neg(&ClassExpr::Class(n.clone())))? {
                top.insert(n.clone());
            }
        }
        let sat: Vec<&Iri> = names.iter().filter(|n| !unsat.contains(*n) && !top.contains(*n)).collect();
        // Is `a ⊑ b`? A model of `a` that lacks `b` refutes it; `b` forced without any choice proves it; only "present, but after
        // a choice" (or a lazily defined `b`) needs a real test.
        // Every model found for `a ⊓ ¬b` also refutes `x ⊑ b` for each named class `x` its node satisfies, which spares those tests.
        let non_sub: std::cell::RefCell<BTreeSet<(Iri, Iri)>> = Default::default();
        let harvest = |m: &State, node: NodeId, b: &Iri| {
            let (at, _) = m.resolve_deps(node);
            for k in m.nodes[at].label.keys() {
                if let ClassExpr::Class(x) = k {
                    non_sub.borrow_mut().insert((x.clone(), b.clone()));
                }
            }
        };
        let test = |a: &Iri, b: &Iri| -> Res<bool> {
            let q = ClassExpr::And(vec![ClassExpr::Class(a.clone()), neg(&ClassExpr::Class(b.clone()))]);
            match self.concept_model(&q)? {
                None => Ok(true),
                Some((m, node)) => {
                    harvest(&m, node, b);
                    Ok(false)
                }
            }
        };
        let cheap_refutation = |a: &Iri, b: &Iri, extra: &ClassExpr| -> bool {
            let Some((m, node)) = states.get(a) else { return false };
            let mut init = m.clone();
            let (at, via) = init.resolve_deps(*node);
            init.add_concept(at, extra.clone(), via);
            let offset = init.max_level;
            match self.search(init, Vec::new(), offset, 3_000, false, &|_| {}) {
                Ok((Some(found), _)) => {
                    harvest(&found, *node, b);
                    true
                }
                _ => false,
            }
        };
        let decide = |a: &Iri, b: &Iri| -> Res<bool> {
            let label = &models[a];
            let target = ClassExpr::Class(b.clone());
            let decided = label.get(&target);
            if decided.is_none() {
                // a lazily defined class is not decided in every node, so "not in the label" refutes nothing: test it,
                // unless the label says outright that the model is not in it
                if !self.tbox.lazy.contains(b) || label.contains_key(&ClassExpr::Not(Box::new(target.clone()))) {
                    return Ok(false);
                }
                // a cheap refutation: the model of `a` fails a named conjunct of the definition of `b`
                if self.model_fails_definition(label, b) {
                    return Ok(false);
                }
                // (first the cheap proof that it does not hold: the model of `a` takes `¬b` as well)
                return if cheap_refutation(a, b, &neg(&target)) { Ok(false) } else { test(a, b) };
            }
            if decided == Some(&true) {
                return Ok(true);
            }
            if cheap_refutation(a, b, &neg(&target)) {
                return Ok(false);
            }
            test(a, b)
        };
        // Classes are taken fewest-told-superclasses first, so a class's superclasses are usually classified before it. That
        // makes two inferences available that save most of the tests: `a ⊑ c` and `c ⊑ b` give `a ⊑ b` (a finished row of `c`
        // is added at once), and `a ⋢ c` with `b ⊑ c` gives `a ⋢ b` (so a refuted class refutes everything known to lie below it).
        let told = self.told_supers();
        let mut order: Vec<&Iri> = sat.clone();
        order.sort_by_key(|n| (told.get(*n).map_or(0, |t| t.len()), (*n).clone()));
        let in_sat: BTreeSet<&Iri> = sat.iter().copied().collect();
        let mut supers: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
        let mut tests = 0usize;
        for a in &order {
            let known = |c: &Iri, supers: &BTreeMap<Iri, BTreeSet<Iri>>| -> BTreeSet<Iri> {
                let mut k: BTreeSet<Iri> = told.get(c).into_iter().flatten().filter(|x| in_sat.contains(x)).cloned().collect();
                if let Some(row) = supers.get(c) {
                    k.extend(row.iter().cloned());
                }
                k
            };
            let mut s: BTreeSet<Iri> = known(a, &supers);
            let mut refuted: BTreeSet<Iri> = BTreeSet::new();
            for b in &order {
                if a == b || s.contains(*b) || refuted.contains(*b) {
                    continue;
                }
                if non_sub.borrow().contains(&((*a).clone(), (*b).clone())) || known(b, &supers).iter().any(|c| refuted.contains(c)) {
                    refuted.insert((*b).clone());
                    continue;
                }
                tests += 1;
                if decide(a, b)? {
                    s.insert((*b).clone());
                    s.extend(known(b, &supers));
                } else {
                    refuted.insert((*b).clone());
                }
            }
            s.remove(*a);
            supers.insert((*a).clone(), s);
        }
        if self.trace.is_some() {
            eprintln!("classify: {tests} subsumption decisions for {} classes (all pairs would be {})", sat.len(), sat.len() * sat.len().saturating_sub(1));
        }
        // equivalence groups
        let mut groups: Vec<BTreeSet<Iri>> = Vec::new();
        let mut seen: BTreeSet<Iri> = BTreeSet::new();
        for a in &sat {
            if seen.contains(*a) {
                continue;
            }
            let mut g = BTreeSet::new();
            g.insert((*a).clone());
            for b in &sat {
                if a != b && supers[*a].contains(*b) && supers[*b].contains(*a) {
                    g.insert((*b).clone());
                }
            }
            seen.extend(g.iter().cloned());
            if g.len() > 1 {
                groups.push(g);
            }
        }
        let rep = |i: &Iri| -> Iri { groups.iter().find(|g| g.contains(i)).map(|g| g.iter().next().unwrap().clone()).unwrap_or_else(|| i.clone()) };
        let mut direct: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
        for a in &sat {
            let strict: BTreeSet<Iri> = supers[*a].iter().filter(|b| !supers[*b].contains(*a)).map(&rep).collect();
            let mut d = BTreeSet::new();
            for b in &strict {
                let dominated = strict.iter().any(|c| c != b && supers.get(c).is_some_and(|sc| sc.contains(b)) && !supers.get(b).is_some_and(|sb| sb.contains(c)));
                if !dominated {
                    d.insert(b.clone());
                }
            }
            direct.insert((*a).clone(), d);
        }
        for t in &top {
            supers.insert(t.clone(), BTreeSet::new());
            direct.insert(t.clone(), BTreeSet::new());
        }
        Ok(Classification { unsatisfiable: unsat, top_equivalents: top, equivalents: groups, all_supers: supers, direct_supers: direct })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// classes equivalent to owl:Nothing
    pub unsatisfiable: BTreeSet<Iri>,
    /// satisfiable classes equivalent to owl:Thing; they appear in no superclass set and form no equivalence group
    pub top_equivalents: BTreeSet<Iri>,
    /// groups of two or more equivalent classes
    pub equivalents: Vec<BTreeSet<Iri>>,
    /// every named superclass of each satisfiable class (excluding itself and owl:Thing)
    pub all_supers: BTreeMap<Iri, BTreeSet<Iri>>,
    /// the closest named superclasses (empty = directly under owl:Thing); equivalent classes appear as their first member
    pub direct_supers: BTreeMap<Iri, BTreeSet<Iri>>,
}
