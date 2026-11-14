// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Property chains (complex role inclusion axioms, `S1 ∘ … ∘ Sn ⊑ R`) for the tableau.
//!
//! A chain makes `R`-paths that are not single edges, so `∀R.C` must reach `C` along every path whose edge roles spell a word
//! of the language of `R`. For a *regular* set of role inclusions that language is regular, and the usual construction
//! (Horrocks and Sattler, *Decidability of SHIQ with complex role inclusion axioms*) builds a finite automaton `B_R` for it:
//! `∀R.C` becomes `∀B_R^q.C` for the automaton state `q`, a concept that moves along one edge and changes state. Only the
//! finitely many state concepts ever appear, which keeps blocking sound.
//!
//! Regularity (what the W3C specification asks of the role hierarchy so that reasoning stays decidable): the roles are ordered
//! so that every chain `S1 … Sn ⊑ R` has each `Si` strictly below `R`, except that `R` may stand first or last
//! (`R S2 … Sn ⊑ R`, `S1 … Sn-1 R ⊑ R`) or be the whole of a two-role chain (`R R ⊑ R`, which is transitivity); sub-property
//! axioms count towards the order too. Roles that are equivalent through the hierarchy are one role here.
//!
//! The automaton for a role is built from the pieces for the roles below it, each copied with its own start and end state so
//! that the loops of one role cannot be taken after another role's loops.

use crate::model::ObjectProp as Role;
use std::collections::{BTreeMap, BTreeSet};

/// An automaton without silent moves: state 0 is the start, and a word is accepted in a state flagged `accepting`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Auto {
    /// per state, the moves: an edge whose role is a sub-role of the label leads to the other state
    pub moves: Vec<Vec<(Role, u32)>>,
    pub accepting: Vec<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct Chains {
    pub autos: Vec<Auto>,
    /// the automaton of every role whose language is more than its own edges (a role with a chain among its sub-roles)
    pub of: BTreeMap<Role, u32>,
}

enum Form {
    /// `S1 … Sn ⊑ R`, no `Si` equivalent to `R`
    General(Vec<Role>),
    /// `R S2 … Sn ⊑ R`: after a path for `R`, the rest leads to another path for `R`
    Left(Vec<Role>),
    /// `S1 … Sn-1 R ⊑ R`
    Right(Vec<Role>),
    /// `R R ⊑ R`
    Both,
}

#[derive(Default)]
struct Nfa {
    eps: Vec<Vec<usize>>,
    edges: Vec<Vec<(Role, usize)>>,
}

impl Nfa {
    fn state(&mut self) -> usize {
        self.eps.push(Vec::new());
        self.edges.push(Vec::new());
        self.eps.len() - 1
    }
}

struct Ctx<'a> {
    class: &'a BTreeMap<Role, Role>,
    direct_subs: &'a BTreeMap<Role, Vec<Role>>,
    forms: &'a BTreeMap<Role, Vec<Form>>,
}

impl Ctx<'_> {
    fn class_of(&self, r: &Role) -> Role {
        self.class.get(r).cloned().unwrap_or_else(|| r.clone())
    }

    /// The part of the automaton of role class `t` between `s` and `e`.
    fn build(&self, nfa: &mut Nfa, t: &Role, s: usize, e: usize) {
        nfa.edges[s].push((t.clone(), e));
        for sub in self.direct_subs.get(t).into_iter().flatten() {
            self.piece(nfa, sub, s, e);
        }
        for form in self.forms.get(t).into_iter().flatten() {
            match form {
                Form::General(w) => self.path(nfa, w, s, e),
                Form::Left(rest) => self.path(nfa, rest, e, e),
                Form::Right(init) => self.path(nfa, init, s, s),
                Form::Both => nfa.eps[e].push(s),
            }
        }
    }

    /// `role` between `s` and `e` as a separate piece (fresh states, joined by silent moves)
    fn piece(&self, nfa: &mut Nfa, role: &Role, s: usize, e: usize) {
        let (s2, e2) = (nfa.state(), nfa.state());
        nfa.eps[s].push(s2);
        nfa.eps[e2].push(e);
        let c = self.class_of(role);
        self.build(nfa, &c, s2, e2);
    }

    /// the roles of `w` one after the other from `s` to `e`
    fn path(&self, nfa: &mut Nfa, w: &[Role], s: usize, e: usize) {
        let mut cur = s;
        for (i, r) in w.iter().enumerate() {
            let next = if i + 1 == w.len() { e } else { nfa.state() };
            self.piece(nfa, r, cur, next);
            cur = next;
        }
    }
}

/// The automata for the chains `chains` (target last), given the role hierarchy `sup` (every role to the roles above it, itself
/// included, inverses mirrored) and the transitive roles. Errors describe a violation of regularity.
pub fn build(sup: &BTreeMap<Role, BTreeSet<Role>>, chains: &[(Vec<Role>, Role)], trans: &BTreeSet<Role>) -> Result<Chains, String> {
    if chains.is_empty() {
        return Ok(Chains::default());
    }
    // every inclusion with its mirror image for the inverse roles; transitivity is `R R ⊑ R`
    let mut rias: Vec<(Vec<Role>, Role)> = Vec::new();
    let mut from_chain: BTreeSet<Role> = BTreeSet::new();
    for (w, t) in chains {
        rias.push((w.clone(), t.clone()));
        rias.push((w.iter().rev().map(Role::inverse).collect(), t.inverse()));
        from_chain.insert(t.clone());
        from_chain.insert(t.inverse());
    }
    for t in trans {
        rias.push((vec![t.clone(), t.clone()], t.clone()));
    }
    rias.sort();
    rias.dedup();

    let mut universe: BTreeSet<Role> = BTreeSet::new();
    for (r, ups) in sup {
        universe.insert(r.clone());
        universe.extend(ups.iter().cloned());
    }
    for (w, t) in &rias {
        universe.extend(w.iter().cloned());
        universe.insert(t.clone());
        universe.insert(t.inverse());
    }
    let universe: Vec<Role> = universe.into_iter().collect();
    let sub = |r: &Role, s: &Role| r == s || sup.get(r).is_some_and(|x| x.contains(s));
    // equivalent roles are one role: the smallest of them stands for all
    let class: BTreeMap<Role, Role> = universe.iter().map(|r| (r.clone(), universe.iter().filter(|x| sub(r, x) && sub(x, r)).min().cloned().unwrap_or_else(|| r.clone()))).collect();
    let reps: BTreeSet<Role> = class.values().cloned().collect();

    let mut forms: BTreeMap<Role, Vec<Form>> = BTreeMap::new();
    let mut below: BTreeMap<Role, BTreeSet<Role>> = BTreeMap::new(); // role class -> the classes it depends on
    for (w, t) in &rias {
        let tc = &class[t];
        let eq: Vec<bool> = w.iter().map(|r| &class[r] == tc).collect();
        let n = w.len();
        let form = if n == 2 && eq[0] && eq[1] {
            Form::Both
        } else if eq[0] && !eq[1..].iter().any(|b| *b) {
            Form::Left(w[1..].to_vec())
        } else if eq[n - 1] && !eq[..n - 1].iter().any(|b| *b) {
            Form::Right(w[..n - 1].to_vec())
        } else if !eq.iter().any(|b| *b) {
            Form::General(w.clone())
        } else {
            return Err(format!("the property chain axiom for `{}` is not regular: the property occurs in it somewhere other than first or last", t.named().local()));
        };
        for (r, e) in w.iter().zip(&eq) {
            if !e {
                below.entry(tc.clone()).or_default().insert(class[r].clone());
            }
        }
        forms.entry(tc.clone()).or_default().push(form);
    }
    // the sub-property relation counts towards the order as well
    for a in &reps {
        for b in &reps {
            if a != b && sub(a, b) {
                below.entry(b.clone()).or_default().insert(a.clone());
            }
        }
    }
    // the order must not have a cycle
    {
        let mut state: BTreeMap<&Role, u8> = BTreeMap::new();
        fn visit<'a>(r: &'a Role, below: &'a BTreeMap<Role, BTreeSet<Role>>, state: &mut BTreeMap<&'a Role, u8>) -> Option<&'a Role> {
            match state.get(r) {
                Some(1) => return Some(r),
                Some(_) => return None,
                None => {}
            }
            state.insert(r, 1);
            for s in below.get(r).into_iter().flatten() {
                if let Some(c) = visit(s, below, state) {
                    return Some(c);
                }
            }
            state.insert(r, 2);
            None
        }
        for r in below.keys() {
            if let Some(c) = visit(r, &below, &mut state) {
                return Err(format!("the property chain axioms are not regular: `{}` depends on itself through chains and sub-properties", c.named().local()));
            }
        }
    }
    // the classes directly below a class: those that are below it but not below another class below it
    let mut direct_subs: BTreeMap<Role, Vec<Role>> = BTreeMap::new();
    for t in &reps {
        let subs: Vec<&Role> = reps.iter().filter(|a| *a != t && sub(a, t)).collect();
        let direct: Vec<Role> = subs.iter().filter(|a| !subs.iter().any(|c| c != *a && sub(a, c))).map(|a| (*a).clone()).collect();
        if !direct.is_empty() {
            direct_subs.insert(t.clone(), direct);
        }
    }
    let ctx = Ctx { class: &class, direct_subs: &direct_subs, forms: &forms };

    let chain_classes: BTreeSet<Role> = from_chain.iter().map(|t| class[t].clone()).collect();
    let mut out = Chains::default();
    let mut made: BTreeMap<Role, u32> = BTreeMap::new();
    for r in &universe {
        let c = &class[r];
        if !chain_classes.iter().any(|s| sub(s, c)) {
            continue;
        }
        let idx = match made.get(c) {
            Some(i) => *i,
            None => {
                let mut nfa = Nfa::default();
                let (s, e) = (nfa.state(), nfa.state());
                ctx.build(&mut nfa, c, s, e);
                out.autos.push(finish(&nfa, s, e));
                let i = (out.autos.len() - 1) as u32;
                made.insert(c.clone(), i);
                i
            }
        };
        out.of.insert(r.clone(), idx);
    }
    Ok(out)
}

/// Remove the silent moves and the states the start cannot reach.
fn finish(nfa: &Nfa, start: usize, end: usize) -> Auto {
    let n = nfa.eps.len();
    let closure = |q: usize| -> Vec<usize> {
        let mut seen = vec![false; n];
        let mut todo = vec![q];
        seen[q] = true;
        while let Some(x) = todo.pop() {
            for &y in &nfa.eps[x] {
                if !seen[y] {
                    seen[y] = true;
                    todo.push(y);
                }
            }
        }
        (0..n).filter(|i| seen[*i]).collect()
    };
    let mut moves: Vec<Vec<(Role, usize)>> = vec![Vec::new(); n];
    let mut accepting = vec![false; n];
    for q in 0..n {
        for p in closure(q) {
            accepting[q] |= p == end;
            for m in &nfa.edges[p] {
                if !moves[q].contains(m) {
                    moves[q].push(m.clone());
                }
            }
        }
    }
    // renumber what the start reaches, the start first
    let mut id: BTreeMap<usize, u32> = BTreeMap::new();
    let mut order = vec![start];
    id.insert(start, 0);
    let mut i = 0;
    while i < order.len() {
        let q = order[i];
        i += 1;
        for (_, to) in &moves[q] {
            if !id.contains_key(to) {
                id.insert(*to, order.len() as u32);
                order.push(*to);
            }
        }
    }
    Auto { moves: order.iter().map(|q| moves[*q].iter().map(|(r, to)| (r.clone(), id[to])).collect()).collect(), accepting: order.iter().map(|q| accepting[*q]).collect() }
}
