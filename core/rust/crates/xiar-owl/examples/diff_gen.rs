// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Differential-testing generator (developer tool). Writes `count` random profile ontologies starting at `seed` into a
//! directory, each as `<n>.ofn` (functional syntax), `<n>.owl` (RDF/XML for the oracle) and `<n>.ours.json` (this
//! reasoner's verdicts). A separate script compares `<n>.ours.json` with the oracle's `<n>.oracle.json`.
//!
//! usage: cargo run --release -p xiar-owl --example diff_gen -- <dir> <seed> <count> [big|nominals]
//!
//! `nominals` mixes `ObjectOneOf` / `ObjectHasValue` into the class expressions on top of inverse roles and number restrictions
//! (the combination that needs the SHOIQ machinery). `keys` is a corpus built around `HasKey`. `chains` is `props` with many regular property chains. `props` adds keys, reflexive, irreflexive, asymmetric and disjoint properties, negative
//! property assertions, `ObjectHasSelf` and `DisjointUnion`.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use xiar_owl::functional::{parse, write};
use xiar_owl::model::*;
use xiar_owl::rdfxml::export;
use xiar_owl::reasoner::{ReasonError, Reasoner};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

const NS: &str = "http://e/#";

/// the `props` mode
static PROPS: AtomicBool = AtomicBool::new(false);
/// the `chains` mode: `props` with many property chains, and no cardinalities or functional axioms (they need simple roles)
static CHAINS: AtomicBool = AtomicBool::new(false);

fn class(i: usize) -> ClassExpr {
    ClassExpr::class(&format!("{NS}C{i}"))
}

fn role(r: &mut Rng, roles: usize) -> ObjectProp {
    let i = Iri::new(format!("{NS}r{}", r.below(roles)));
    if r.chance(25) { ObjectProp::Inverse(i) } else { ObjectProp::Named(i) }
}

fn ind(i: usize) -> Iri {
    Iri::new(format!("{NS}i{i}"))
}

fn expr(r: &mut Rng, classes: usize, roles: usize, depth: usize) -> ClassExpr {
    expr_n(r, classes, roles, 0, depth)
}

/// `inds > 0` allows nominals over the individuals `i0..i{inds-1}`.
fn expr_n(r: &mut Rng, classes: usize, roles: usize, inds: usize, depth: usize) -> ClassExpr {
    if depth == 0 || r.chance(35) {
        return class(r.below(classes));
    }
    let d = depth - 1;
    if inds > 0 && r.chance(22) {
        return if r.chance(50) {
            let mut m = vec![ind(r.below(inds))];
            if r.chance(40) {
                m.push(ind(r.below(inds)));
            }
            m.sort();
            m.dedup();
            ClassExpr::OneOf(m)
        } else {
            ClassExpr::HasValue(role(r, roles), ind(r.below(inds)))
        };
    }
    if PROPS.load(Ordering::Relaxed) && r.chance(9) {
        return ClassExpr::HasSelf(role(r, roles));
    }
    match if CHAINS.load(Ordering::Relaxed) { r.below(6) } else { r.below(9) } {
        0 => ClassExpr::And(vec![expr_n(r, classes, roles, inds, d), expr_n(r, classes, roles, inds, d)]),
        1 => ClassExpr::Or(vec![expr_n(r, classes, roles, inds, d), expr_n(r, classes, roles, inds, d)]),
        2 => ClassExpr::Not(Box::new(expr_n(r, classes, roles, inds, d))),
        3 | 4 => ClassExpr::Some(role(r, roles), Box::new(expr_n(r, classes, roles, inds, d))),
        5 => ClassExpr::All(role(r, roles), Box::new(expr_n(r, classes, roles, inds, d))),
        6 => ClassExpr::Min(1 + r.below(3) as u32, role(r, roles), Box::new(if r.chance(50) { ClassExpr::Thing } else { expr_n(r, classes, roles, inds, d) })),
        7 => ClassExpr::Max(r.below(3) as u32, role(r, roles), Box::new(if r.chance(50) { ClassExpr::Thing } else { expr_n(r, classes, roles, inds, d) })),
        _ => ClassExpr::Exact(1 + r.below(2) as u32, role(r, roles), Box::new(expr_n(r, classes, roles, inds, d))),
    }
}

/// The `keys` mode: several individuals of one class that point at few shared neighbours, with a key on that class, and class
/// assertions that clash once two of them are identified.
fn generate_keys(seed: u64) -> Ontology {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for _ in 0..5 {
        r.next();
    }
    let classes = 3 + r.below(3);
    let inds = 4 + r.below(4);
    let shared = 2;
    let mut o = Ontology { iri: Some(Iri::new("http://e")), ..Ontology::default() };
    for i in 0..classes {
        o.axioms.push(Axiom::DeclareClass(Iri::new(format!("{NS}C{i}"))));
    }
    for i in 0..2 {
        o.axioms.push(Axiom::DeclareObjectProperty(Iri::new(format!("{NS}r{i}"))));
    }
    let rp = |i: usize| ObjectProp::Named(Iri::new(format!("{NS}r{i}")));
    let mut keys = vec![if r.chance(25) { rp(0).inverse() } else { rp(0) }];
    if r.chance(35) {
        keys.push(rp(1));
    }
    o.axioms.push(Axiom::HasKey(class(0), keys, Vec::new()));
    if r.chance(40) {
        o.axioms.push(Axiom::SubObjectPropertyOf(rp(1), rp(0)));
    }
    for _ in 0..r.below(4) {
        let named = class(r.below(classes));
        o.axioms.push(Axiom::SubClassOf(named, expr(&mut r, classes, 2, 1)));
    }
    for i in 0..inds {
        o.axioms.push(Axiom::DeclareIndividual(ind(i)));
        if r.chance(80) {
            o.axioms.push(Axiom::ClassAssertion(class(0), ind(i)));
        }
        if r.chance(45) {
            let c = class(1 + r.below(classes - 1));
            o.axioms.push(Axiom::ClassAssertion(if r.chance(50) { ClassExpr::Not(Box::new(c)) } else { c }, ind(i)));
        }
        for k in 0..2 {
            if r.chance(if k == 0 { 70 } else { 40 }) {
                // the shared neighbours are the last individuals
                let target = if r.chance(80) { inds - 1 - r.below(shared) } else { r.below(inds) };
                o.axioms.push(Axiom::ObjectPropertyAssertion(rp(k), ind(i), ind(target)));
            }
        }
    }
    for _ in 0..r.below(3) {
        let (a, b) = (r.below(inds), r.below(inds));
        if a != b {
            o.axioms.push(Axiom::DifferentIndividuals(vec![ind(a), ind(b)]));
        }
    }
    o
}

fn generate(seed: u64, big: bool, nominals: bool) -> Ontology {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for _ in 0..5 {
        r.next();
    }
    let (classes, roles, mut inds, axioms, depth) = if big { (8 + r.below(8), 2 + r.below(3), r.below(7), 10 + r.below(16), 3) } else { (4 + r.below(4), 1 + r.below(3), r.below(4), 3 + r.below(8), 2) };
    if nominals {
        inds = inds.max(2 + r.below(3));
    }
    let nom = if nominals { inds } else { 0 };
    let mut o = Ontology { iri: Some(Iri::new("http://e")), ..Ontology::default() };
    for i in 0..classes {
        o.axioms.push(Axiom::DeclareClass(Iri::new(format!("{NS}C{i}"))));
    }
    for i in 0..roles {
        o.axioms.push(Axiom::DeclareObjectProperty(Iri::new(format!("{NS}r{i}"))));
    }
    for _ in 0..axioms {
        let named = class(r.below(classes));
        if PROPS.load(Ordering::Relaxed) && r.chance(if CHAINS.load(Ordering::Relaxed) { 55 } else { 30 }) {
            let p = role(&mut r, roles);
            if roles >= 2 && r.chance(if CHAINS.load(Ordering::Relaxed) { 85 } else { 35 }) {
                // a regular chain: the members are roles below the target in index order, the target may recur first or last
                let k = 1 + r.below(roles - 1);
                let low = |r: &mut Rng| {
                    let i = Iri::new(format!("{NS}r{}", r.below(k)));
                    if r.chance(25) { ObjectProp::Inverse(i) } else { ObjectProp::Named(i) }
                };
                let target = ObjectProp::Named(Iri::new(format!("{NS}r{k}")));
                let w = match r.below(4) {
                    0 => vec![target.clone(), low(&mut r)],
                    1 => vec![low(&mut r), target.clone()],
                    _ => (0..2 + r.below(2)).map(|_| low(&mut r)).collect(),
                };
                o.axioms.push(Axiom::SubPropertyChain(w, if r.chance(20) { target.inverse() } else { target }));
                continue;
            }
            if r.chance(18) {
                // a key over one or two roles (simple roles only: the generator drops ontologies where they are not)
                let mut keys = vec![p.clone()];
                if r.chance(30) {
                    keys.push(role(&mut r, roles));
                }
                o.axioms.push(Axiom::HasKey(class(r.below(classes)), keys, Vec::new()));
                continue;
            }
            o.axioms.push(match r.below(5) {
                0 => Axiom::ReflexiveObjectProperty(p),
                1 => Axiom::IrreflexiveObjectProperty(p),
                2 => Axiom::AsymmetricObjectProperty(p),
                3 => Axiom::DisjointObjectProperties(vec![p, role(&mut r, roles)]),
                _ => {
                    let (a, b) = (r.below(classes), r.below(classes));
                    if a == b { continue } else { Axiom::DisjointUnion(Iri::new(format!("{NS}C{}", r.below(classes))), vec![class(a), class(b)]) }
                }
            });
            continue;
        }
        let ax = match r.below(11) {
            0..=3 => Axiom::SubClassOf(named, expr_n(&mut r, classes, roles, nom, depth)),
            4 => Axiom::EquivalentClasses(vec![named, expr_n(&mut r, classes, roles, nom, depth)]),
            5 => Axiom::DisjointClasses(vec![named, class(r.below(classes))]),
            6 => Axiom::ObjectPropertyDomain(role(&mut r, roles), expr(&mut r, classes, roles, 1)),
            7 => Axiom::ObjectPropertyRange(role(&mut r, roles), expr(&mut r, classes, roles, 1)),
            8 => {
                let p = role(&mut r, roles);
                match if CHAINS.load(Ordering::Relaxed) { 2 + r.below(3) } else { r.below(5) } {
                    0 => Axiom::FunctionalObjectProperty(p),
                    1 => Axiom::InverseFunctionalObjectProperty(p),
                    2 => Axiom::TransitiveObjectProperty(p),
                    3 => Axiom::SymmetricObjectProperty(p),
                    _ => Axiom::SubObjectPropertyOf(p, role(&mut r, roles)),
                }
            }
            9 => Axiom::SubClassOf(expr_n(&mut r, classes, roles, nom, 1), expr_n(&mut r, classes, roles, nom, 1)),
            _ => {
                let (a, b) = (r.below(roles), r.below(roles));
                if a == b { Axiom::SymmetricObjectProperty(ObjectProp::Named(Iri::new(format!("{NS}r{a}")))) } else { Axiom::InverseObjectProperties(Iri::new(format!("{NS}r{a}")), Iri::new(format!("{NS}r{b}"))) }
            }
        };
        o.axioms.push(ax);
    }
    for i in 0..inds {
        let ind = Iri::new(format!("{NS}i{i}"));
        o.axioms.push(Axiom::DeclareIndividual(ind.clone()));
        if r.chance(80) {
            o.axioms.push(Axiom::ClassAssertion(expr(&mut r, classes, roles, 1), ind.clone()));
        }
        if inds > 1 && r.chance(60) {
            let other = Iri::new(format!("{NS}i{}", r.below(inds)));
            o.axioms.push(Axiom::ObjectPropertyAssertion(role(&mut r, roles), ind, other));
        }
    }
    if PROPS.load(Ordering::Relaxed) && inds > 1 {
        for _ in 0..r.below(3) {
            o.axioms.push(Axiom::NegativeObjectPropertyAssertion(role(&mut r, roles), ind(r.below(inds)), ind(r.below(inds))));
        }
    }
    if inds > 1 && r.chance(15) {
        o.axioms.push(Axiom::DifferentIndividuals(vec![Iri::new(format!("{NS}i0")), Iri::new(format!("{NS}i1"))]));
    }
    o
}

fn json_list(items: impl Iterator<Item = String>) -> String {
    format!("[{}]", items.map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(","))
}

fn locals(s: &BTreeSet<Iri>) -> BTreeSet<String> {
    s.iter().map(|i| i.0.clone()).collect()
}

fn verdicts(o: &Ontology) -> String {
    let steps: usize = std::env::var("XIAR_OWL_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(2_000_000);
    let r = Reasoner::new(o).with_budget(steps, 200_000);
    match r.is_consistent() {
        Err(ReasonError::Budget(w)) => return format!("{{\"error\":\"budget {w}\"}}"),
        Err(e) => return format!("{{\"error\":\"{e}\"}}"),
        Ok(false) => return "{\"consistent\":false}".to_string(),
        Ok(true) => {}
    }
    let k = match r.classify() {
        Ok(k) => k,
        Err(e) => return format!("{{\"error\":\"{e}\"}}"),
    };
    let supers = k.all_supers.iter().map(|(c, s)| format!("\"{}\":{}", c.0, json_list(locals(s).into_iter()))).collect::<Vec<_>>().join(",");
    let equiv = k.equivalents.iter().map(|g| json_list(locals(g).into_iter())).collect::<Vec<_>>().join(",");
    let mut types = Vec::new();
    for i in o.individual_names() {
        match r.types(&i) {
            Ok(t) => types.push(format!("\"{}\":{}", i.0, json_list(locals(&t).into_iter()))),
            Err(e) => return format!("{{\"error\":\"{e}\"}}"),
        }
    }
    format!("{{\"consistent\":true,\"top\":{},\"unsat\":{},\"supers\":{{{}}},\"equiv\":[{}],\"types\":{{{}}}}}", json_list(locals(&k.top_equivalents).into_iter()), json_list(locals(&k.unsatisfiable).into_iter()), supers, equiv, types.join(","))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: diff_gen <dir> <seed> <count> [big]");
        std::process::exit(2);
    }
    let dir = std::path::Path::new(&args[1]);
    std::fs::create_dir_all(dir).expect("create dir");
    let seed: u64 = args[2].parse().expect("seed");
    let count: u64 = args[3].parse().expect("count");
    let big = args.get(4).is_some_and(|a| a == "big");
    let nominals = args.get(4).is_some_and(|a| a == "nominals");
    let keys_mode = args.get(4).is_some_and(|a| a == "keys");
    PROPS.store(args.get(4).is_some_and(|a| a == "props" || a == "chains"), Ordering::Relaxed);
    CHAINS.store(args.get(4).is_some_and(|a| a == "chains"), Ordering::Relaxed);
    let mut written = 0;
    let mut n = seed;
    while written < count {
        let o = if keys_mode { generate_keys(n) } else { generate(n, big, nominals) };
        n += 1;
        let n = n - 1;
        if xiar_owl::reasoner::check_simple_roles(&o).is_err() {
            continue; // not valid OWL 2 DL; the reader rejects such ontologies
        }
        written += 1;
        let text = write(&o);
        let back = parse(&text).expect("generated ontology must parse");
        assert_eq!(back, o, "seed {n}: write/parse roundtrip");
        std::fs::write(dir.join(format!("{n}.ofn")), &text).unwrap();
        std::fs::write(dir.join(format!("{n}.owl")), export(&o).expect("export")).unwrap();
        std::fs::write(dir.join(format!("{n}.ours.json")), verdicts(&o)).unwrap();
    }
    println!("wrote {count} ontologies to {}", dir.display());
}
