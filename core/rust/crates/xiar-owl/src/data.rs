// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Data values and data ranges for the reasoner: can one individual's data property have a set of values that satisfies every
//! constraint the tableau has collected for it?
//!
//! For one node and one data property the question is: is there a finite set `V` of data values such that
//! * the asserted values are in `V`,
//! * every value in `V` is in every "all values from" range,
//! * `V` has at least `n` values in each range of a "some / has value / min cardinality" constraint, and
//! * `V` has at most `m` values in each range of a "max cardinality" constraint?
//!
//! The value spaces are strings, booleans, date-times (opaque), decimals (the integers among them) and the reals of
//! `xsd:float`/`xsd:double`. Every data range is evaluated exactly on a value; the ranges are *sampled*: the values the
//! constraints mention, the endpoints and boundaries of numeric ranges, and enough extra values inside every region between
//! them. A value only matters through which ranges it lies in (its *cell*), and a cell never needs more than `K` values,
//! where `K` is more than the largest count any constraint can ask for, so counting values per cell on that sample is exact.
//! Cells are then combined by a small bounded search. If a literal cannot be read, or the problem is too large, the answer is
//! an error, never a guess.

use crate::model::*;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    /// something the value-space model does not cover (a malformed literal, too many ranges, numbers too large)
    Unsupported(String),
    /// the search for a value assignment exceeded its work limit
    Budget,
}

type Res<T> = Result<T, DataError>;

// ------------------------------------------------------------------ exact rational numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rat {
    n: i128,
    d: i128,
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Rat {
    fn new(n: i128, d: i128) -> Rat {
        let g = gcd(n, d).max(1);
        let s = if d < 0 { -1 } else { 1 };
        Rat { n: s * n / g, d: s * d / g }
    }

    fn int(n: i128) -> Rat {
        Rat { n, d: 1 }
    }

    fn is_integer(&self) -> bool {
        self.d == 1
    }

    fn add(&self, o: &Rat) -> Res<Rat> {
        let n = self.n.checked_mul(o.d).and_then(|a| o.n.checked_mul(self.d).and_then(|b| a.checked_add(b)));
        let d = self.d.checked_mul(o.d);
        match (n, d) {
            (Some(n), Some(d)) => Ok(Rat::new(n, d)),
            _ => Err(DataError::Unsupported("numbers too large".into())),
        }
    }

    fn scale(&self, num: i128, den: i128) -> Res<Rat> {
        match (self.n.checked_mul(num), self.d.checked_mul(den)) {
            (Some(n), Some(d)) => Ok(Rat::new(n, d)),
            _ => Err(DataError::Unsupported("numbers too large".into())),
        }
    }

    fn neg(&self) -> Rat {
        Rat { n: -self.n, d: self.d }
    }

    /// floor as an integer
    fn floor(&self) -> i128 {
        self.n.div_euclid(self.d)
    }
}

impl PartialOrd for Rat {
    fn partial_cmp(&self, o: &Rat) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Rat {
    fn cmp(&self, o: &Rat) -> std::cmp::Ordering {
        // both denominators are positive; a product that would overflow can only come from numbers this model rejects on parse
        (self.n.saturating_mul(o.d)).cmp(&o.n.saturating_mul(self.d))
    }
}

/// Parse an xsd decimal / integer / float lexical form (`-12`, `3.50`, `1.5E2`).
fn parse_number(s: &str) -> Res<Rat> {
    let t = s.trim();
    let bad = || DataError::Unsupported(format!("cannot read the number `{s}`"));
    if t.is_empty() || t.len() > 38 {
        return Err(bad());
    }
    let (mantissa, exp) = match t.find(['e', 'E']) {
        Some(i) => (&t[..i], t[i + 1..].parse::<i32>().map_err(|_| bad())?),
        None => (t, 0),
    };
    if exp.abs() > 18 {
        return Err(bad());
    }
    let (neg, digits) = match mantissa.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, mantissa.strip_prefix('+').unwrap_or(mantissa)),
    };
    let (int_part, frac_part) = digits.split_once('.').unwrap_or((digits, ""));
    if (int_part.is_empty() && frac_part.is_empty()) || !int_part.chars().chain(frac_part.chars()).all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    let all = format!("{int_part}{frac_part}");
    if all.len() > 30 {
        return Err(bad());
    }
    let mut n: i128 = all.parse().map_err(|_| bad())?;
    let mut d: i128 = 10i128.pow(frac_part.len() as u32);
    if exp > 0 {
        n = n.checked_mul(10i128.pow(exp as u32)).ok_or_else(bad)?;
    } else if exp < 0 {
        d = d.checked_mul(10i128.pow((-exp) as u32)).ok_or_else(bad)?;
    }
    Ok(Rat::new(if neg { -n } else { n }, d))
}

// ------------------------------------------------------------------ values
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Val {
    Str(String),
    Bool(bool),
    Time(String),
    /// a decimal (every integer is one)
    Dec(Rat),
    /// a float / double
    Dbl(Rat),
}

/// The value a literal denotes.
pub fn value_of(l: &Literal) -> Res<Val> {
    let dt = l.datatype;
    Ok(match dt {
        Datatype::String => Val::Str(l.lexical.clone()),
        Datatype::Boolean => match l.lexical.trim() {
            "true" | "1" => Val::Bool(true),
            "false" | "0" => Val::Bool(false),
            other => return Err(DataError::Unsupported(format!("`{other}` is not a boolean"))),
        },
        Datatype::DateTime => Val::Time(l.lexical.trim().to_string()),
        Datatype::Float | Datatype::Double => Val::Dbl(parse_number(&l.lexical)?),
        _ => {
            let r = parse_number(&l.lexical)?;
            if dt.is_integer() && !r.is_integer() {
                return Err(DataError::Unsupported(format!("`{}` is not an integer", l.lexical)));
            }
            Val::Dec(r)
        }
    })
}

/// Closed bounds an integer datatype imposes (`None` = unbounded on that side).
fn integer_bounds(dt: Datatype) -> (Option<i128>, Option<i128>) {
    match dt {
        Datatype::NonNegativeInteger => (Some(0), None),
        Datatype::PositiveInteger => (Some(1), None),
        Datatype::NonPositiveInteger => (None, Some(0)),
        Datatype::NegativeInteger => (None, Some(-1)),
        Datatype::Long => (Some(i64::MIN as i128), Some(i64::MAX as i128)),
        Datatype::Int => (Some(i32::MIN as i128), Some(i32::MAX as i128)),
        Datatype::Short => (Some(i16::MIN as i128), Some(i16::MAX as i128)),
        Datatype::Byte => (Some(i8::MIN as i128), Some(i8::MAX as i128)),
        _ => (None, None),
    }
}

fn in_datatype(dt: Datatype, v: &Val) -> bool {
    match (dt, v) {
        (Datatype::String, Val::Str(_)) | (Datatype::Boolean, Val::Bool(_)) | (Datatype::DateTime, Val::Time(_)) => true,
        (Datatype::Decimal, Val::Dec(_)) => true,
        (Datatype::Float | Datatype::Double, Val::Dbl(_)) => true,
        (t, Val::Dec(r)) if t.is_integer() => {
            let (lo, hi) = integer_bounds(t);
            r.is_integer() && lo.is_none_or(|l| r.n >= l) && hi.is_none_or(|h| r.n <= h)
        }
        _ => false,
    }
}

/// A data range with its literals already turned into values.
#[derive(Debug, Clone)]
enum R {
    Top,
    Datatype(Datatype),
    OneOf(Vec<Val>),
    Not(Box<R>),
    And(Vec<R>),
    Or(Vec<R>),
    Facets(Datatype, Vec<(Facet, Rat)>),
}

fn compile(d: &DataRange) -> Res<R> {
    Ok(match d {
        DataRange::Top => R::Top,
        DataRange::Datatype(t) => R::Datatype(*t),
        DataRange::OneOf(v) => R::OneOf(v.iter().map(value_of).collect::<Res<_>>()?),
        DataRange::Not(x) => R::Not(Box::new(compile(x)?)),
        DataRange::And(v) => R::And(v.iter().map(compile).collect::<Res<_>>()?),
        DataRange::Or(v) => R::Or(v.iter().map(compile).collect::<Res<_>>()?),
        DataRange::Facets(t, fs) => {
            let mut out = Vec::new();
            for (f, l) in fs {
                match value_of(l)? {
                    Val::Dec(r) | Val::Dbl(r) => out.push((*f, r)),
                    _ => return Err(DataError::Unsupported("a facet value must be a number".into())),
                }
            }
            R::Facets(*t, out)
        }
    })
}

fn eval(r: &R, v: &Val) -> bool {
    match r {
        R::Top => true,
        R::Datatype(t) => in_datatype(*t, v),
        R::OneOf(vs) => vs.contains(v),
        R::Not(x) => !eval(x, v),
        R::And(xs) => xs.iter().all(|x| eval(x, v)),
        R::Or(xs) => xs.iter().any(|x| eval(x, v)),
        R::Facets(t, fs) => {
            if !in_datatype(*t, v) {
                return false;
            }
            let (Val::Dec(x) | Val::Dbl(x)) = v else { return false };
            fs.iter().all(|(f, b)| match f {
                Facet::MinInclusive => x >= b,
                Facet::MaxInclusive => x <= b,
                Facet::MinExclusive => x > b,
                Facet::MaxExclusive => x < b,
            })
        }
    }
}

/// The numbers where a range's membership can change (its endpoints, an integer datatype's bounds, its literals).
fn breakpoints(r: &R, dec: &mut BTreeSet<Rat>, dbl: &mut BTreeSet<Rat>, others: &mut BTreeSet<Val>) {
    match r {
        R::Top => {}
        R::Datatype(t) => {
            let (lo, hi) = integer_bounds(*t);
            dec.extend(lo.map(Rat::int));
            dec.extend(hi.map(Rat::int));
        }
        R::OneOf(vs) => {
            for v in vs {
                match v {
                    Val::Dec(r) => {
                        dec.insert(*r);
                    }
                    Val::Dbl(r) => {
                        dbl.insert(*r);
                    }
                    other => {
                        others.insert(other.clone());
                    }
                }
            }
        }
        R::Not(x) => breakpoints(x, dec, dbl, others),
        R::And(xs) | R::Or(xs) => xs.iter().for_each(|x| breakpoints(x, dec, dbl, others)),
        R::Facets(t, fs) => {
            let (lo, hi) = integer_bounds(*t);
            let space = if matches!(t, Datatype::Float | Datatype::Double) { &mut *dbl } else { &mut *dec };
            space.extend(lo.map(Rat::int));
            space.extend(hi.map(Rat::int));
            space.extend(fs.iter().map(|(_, b)| *b));
        }
    }
}

/// Sample numbers: every breakpoint, and `k` integers and `k` non-integers in each region between them and beyond them.
fn number_samples(points: &BTreeSet<Rat>, k: usize) -> Res<Vec<Rat>> {
    let pts: Vec<Rat> = points.iter().copied().collect();
    let mut out: Vec<Rat> = pts.clone();
    // regions: (-inf, p0), (p0, p1), ..., (pm, +inf)
    let mut regions: Vec<(Option<Rat>, Option<Rat>)> = Vec::new();
    if pts.is_empty() {
        regions.push((None, None));
    } else {
        regions.push((None, Some(pts[0])));
        for w in pts.windows(2) {
            regions.push((Some(w[0]), Some(w[1])));
        }
        regions.push((Some(*pts.last().unwrap()), None));
    }
    for (lo, hi) in regions {
        // integers strictly inside the region
        let first = match lo {
            Some(l) => l.floor() + 1,
            None => match hi {
                Some(h) => h.floor() - k as i128 - 1,
                None => 0,
            },
        };
        let mut z = first;
        let mut taken = 0;
        while taken < k {
            let r = Rat::int(z);
            if lo.is_some_and(|l| r <= l) {
                z += 1;
                continue;
            }
            if hi.is_some_and(|h| r >= h) {
                break;
            }
            out.push(r);
            taken += 1;
            z += 1;
        }
        // non-integers: evenly spread, away from the integers by using an odd denominator
        let (a, b) = match (lo, hi) {
            (Some(l), Some(h)) => (l, h),
            (Some(l), None) => (l, l.add(&Rat::int(k as i128 + 2))?),
            (None, Some(h)) => (h.add(&Rat::int(-(k as i128) - 2))?, h),
            (None, None) => (Rat::int(0), Rat::int(k as i128 + 2)),
        };
        let width = b.add(&a.neg())?;
        let q = 2 * k as i128 + 3;
        let mut got = 0;
        let mut j = 1;
        while got < k && j < 4 * q {
            let x = a.add(&width.scale(j, q)?)?;
            if !x.is_integer() && x > a && x < b {
                out.push(x);
                got += 1;
            }
            j += 1;
        }
    }
    Ok(out)
}

/// The most ranges one problem may mention (cells are tracked as bits of a `u128`).
const MAX_RANGES: usize = 120;

/// Is there a set of values meeting all the constraints? See the module documentation.
pub fn feasible(vals: &[Literal], alls: &[&DataRange], exist: &[(u32, &DataRange)], maxes: &[(u32, &DataRange)]) -> Res<bool> {
    feasible_group(&[Prop { vals, alls, exist, maxes }], &[1], &[])
}

/// What one data property of a node must satisfy (the "all values" ranges include those of its super-properties).
pub struct Prop<'a> {
    pub vals: &'a [Literal],
    pub alls: &'a [&'a DataRange],
    pub exist: &'a [(u32, &'a DataRange)],
    pub maxes: &'a [(u32, &'a DataRange)],
}

/// Most properties of one hierarchy component that one node can use at once.
const MAX_GROUP: usize = 8;

/// A group of properties related by sub-property axioms. `sup[i]` is the bit set of the properties (in the group, `i` itself
/// included) that property `i` is a sub-property of. A value belongs to a set of properties closed upwards, and counts for the
/// minimum and maximum of every property in that set; the sets are enumerated, so nothing is guessed. `disjoint` lists masks of
/// properties no value may belong to together (a single bit: that property has no value at all).
pub fn feasible_group(props: &[Prop], sup: &[u64], disjoint: &[u64]) -> Res<bool> {
    let np = props.len();
    if np > MAX_GROUP {
        return Err(DataError::Unsupported("too many related data properties on one node".into()));
    }
    let u: Vec<Vec<R>> = props.iter().map(|p| p.alls.iter().map(|d| compile(d)).collect::<Res<_>>()).collect::<Res<_>>()?;
    // counted ranges, each tied to its property
    let mut ex: Vec<(usize, u32, R)> = Vec::new();
    let mut mx: Vec<(usize, u32, R)> = Vec::new();
    for (i, p) in props.iter().enumerate() {
        for (n, d) in p.exist {
            ex.push((i, *n, compile(d)?));
        }
        for (m, d) in p.maxes {
            mx.push((i, *m, compile(d)?));
        }
    }
    if ex.len() + mx.len() > MAX_RANGES {
        return Err(DataError::Unsupported("too many data ranges on one property".into()));
    }
    // asserted values with the properties they were asserted for
    let mut forced: Vec<(Val, u64)> = Vec::new();
    for (i, p) in props.iter().enumerate() {
        for l in p.vals {
            let v = value_of(l)?;
            match forced.iter_mut().find(|(w, _)| *w == v) {
                Some((_, m)) => *m |= sup[i],
                None => forced.push((v, sup[i])),
            }
        }
    }
    let need: u64 = ex.iter().map(|(_, n, _)| *n as u64).sum::<u64>() + mx.iter().map(|(_, m, _)| *m as u64 + 1).sum::<u64>() + forced.len() as u64 + 2;
    if need > 400 {
        return Err(DataError::Unsupported("data cardinalities too large".into()));
    }
    let k = need as usize;
    // the property sets a value may belong to: non-empty and closed upwards
    // (properties that are disjoint, as bit masks, cannot share a value: no set holds all the bits of such a mask)
    let sets: Vec<u64> = (1u64..(1 << np)).filter(|s| (0..np).all(|i| s >> i & 1 == 0 || sup[i] & !s == 0) && disjoint.iter().all(|m| s & m != *m)).collect();
    // a value may belong to a set only if it satisfies the "all values" ranges of every property in it
    let allowed = |v: &Val, s: u64| (0..np).all(|i| s >> i & 1 == 0 || u[i].iter().all(|r| eval(r, v)));
    // asserted values must be allowed in some set that contains their properties
    for (v, req) in &forced {
        if !sets.iter().any(|s| s & req == *req && allowed(v, *s)) {
            return Ok(false);
        }
    }
    // sample the value space
    let (mut dec, mut dbl, mut others) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for r in u.iter().flatten().chain(ex.iter().map(|(_, _, r)| r)).chain(mx.iter().map(|(_, _, r)| r)) {
        breakpoints(r, &mut dec, &mut dbl, &mut others);
    }
    for (v, _) in &forced {
        match v {
            Val::Dec(r) => {
                dec.insert(*r);
            }
            Val::Dbl(r) => {
                dbl.insert(*r);
            }
            other => {
                others.insert(other.clone());
            }
        }
    }
    let mut candidates: Vec<Val> = vec![Val::Bool(true), Val::Bool(false)];
    candidates.extend(others);
    for i in 0..k {
        candidates.push(Val::Str(format!("\u{1}fresh{i}")));
        candidates.push(Val::Time(format!("\u{1}fresh{i}")));
    }
    candidates.extend(number_samples(&dec, k)?.into_iter().map(Val::Dec));
    candidates.extend(number_samples(&dbl, k)?.into_iter().map(Val::Dbl));
    candidates.sort();
    candidates.dedup();

    // the counted ranges a value in property set `s` counts for, as a bit mask (ex first, then mx)
    let counts = |v: &Val, s: u64| -> u128 {
        let mut m = 0u128;
        for (i, (p, _, r)) in ex.iter().enumerate() {
            if s >> p & 1 == 1 && eval(r, v) {
                m |= 1 << i;
            }
        }
        for (i, (p, _, r)) in mx.iter().enumerate() {
            if s >> p & 1 == 1 && eval(r, v) {
                m |= 1 << (ex.len() + i);
            }
        }
        m
    };
    // regions: values that are interchangeable (same allowed property sets, same range membership)
    let membership = |v: &Val| -> (u64, Vec<bool>) {
        let al = sets.iter().enumerate().fold(0u64, |a, (i, s)| if allowed(v, *s) { a | 1 << i } else { a });
        let bits: Vec<bool> = ex.iter().map(|(_, _, r)| eval(r, v)).chain(mx.iter().map(|(_, _, r)| eval(r, v))).collect();
        (al, bits)
    };
    let mut region_of: std::collections::BTreeMap<(u64, Vec<bool>), usize> = std::collections::BTreeMap::new();
    let mut regions: Vec<(Val, usize)> = Vec::new(); // a representative and the number of free values
    for v in &candidates {
        let key = membership(v);
        if key.0 == 0 {
            continue;
        }
        let forced_here = forced.iter().any(|(w, _)| w == v);
        let id = *region_of.entry(key).or_insert_with(|| {
            regions.push((v.clone(), 0));
            regions.len() - 1
        });
        if !forced_here {
            regions[id].1 += 1;
        }
    }
    // each asserted value picks a property set: the alternatives are (count mask, region)
    let mut options: Vec<Vec<(u128, usize)>> = Vec::new();
    for (v, req) in &forced {
        let region = region_of[&membership(v)];
        let mut alts: Vec<(u128, usize)> = Vec::new();
        for s in &sets {
            if s & req == *req && allowed(v, *s) {
                let m = counts(v, *s);
                if !alts.iter().any(|a| a.0 == m) {
                    alts.push((m, region));
                }
            }
        }
        options.push(alts);
    }
    // free cells: per region, one cell per distinct count mask a value of the region can have
    let mut cells: Vec<(u128, usize, usize)> = Vec::new(); // (mask, region, most useful values)
    for (rid, (rep, avail)) in regions.iter().enumerate() {
        let mut masks: Vec<u128> = Vec::new();
        for s in &sets {
            if allowed(rep, *s) {
                let m = counts(rep, *s);
                if !masks.contains(&m) {
                    masks.push(m);
                }
            }
        }
        for m in masks {
            // taking more values than the minimums can use only risks the maximums
            let useful: usize = ex.iter().enumerate().filter(|(j, _)| m >> j & 1 == 1).map(|(_, (_, n, _))| *n as usize).sum();
            if useful > 0 {
                cells.push((m, rid, useful.min(*avail).min(k)));
            }
        }
    }
    let caps: Vec<usize> = regions.iter().map(|r| r.1.min(k)).collect();
    let mut budget = 2_000_000usize;
    let ctx = Search { cells: &cells, ex: &ex, mx: &mx, caps: &caps };
    let mut base = vec![0usize; ex.len() + mx.len()];
    choose(0, &options, &mut base, &ctx, &mut budget)
}

struct Search<'a> {
    cells: &'a [(u128, usize, usize)],
    ex: &'a [(usize, u32, R)],
    mx: &'a [(usize, u32, R)],
    caps: &'a [usize],
}

/// Choose the property set of each asserted value (`base` counts what they give each range), then search the free values.
fn choose(i: usize, options: &[Vec<(u128, usize)>], base: &mut Vec<usize>, ctx: &Search, budget: &mut usize) -> Res<bool> {
    if i == options.len() {
        let mut x = vec![0usize; ctx.cells.len()];
        return go(0, &mut x, base, ctx, budget);
    }
    let bits = ctx.ex.len() + ctx.mx.len();
    for (m, _) in &options[i] {
        for (b, n) in base.iter_mut().enumerate().take(bits) {
            if m >> b & 1 == 1 {
                *n += 1;
            }
        }
        let over = ctx.mx.iter().enumerate().any(|(j, (_, max, _))| base[ctx.ex.len() + j] > *max as usize);
        let ok = !over && choose(i + 1, options, base, ctx, budget)?;
        for (b, n) in base.iter_mut().enumerate().take(bits) {
            if m >> b & 1 == 1 {
                *n -= 1;
            }
        }
        if ok {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Search the number of free values to take from each cell.
fn go(i: usize, x: &mut Vec<usize>, base: &[usize], ctx: &Search, budget: &mut usize) -> Res<bool> {
    if *budget == 0 {
        return Err(DataError::Budget);
    }
    *budget -= 1;
    let cells = ctx.cells;
    let ne = ctx.ex.len();
    // prune: a maximum already exceeded, a region over capacity, or a minimum the remaining cells cannot reach
    for (j, (_, m, _)) in ctx.mx.iter().enumerate() {
        let b = ne + j;
        let sum: usize = base[b] + cells.iter().zip(x.iter()).filter(|(c, _)| c.0 >> b & 1 == 1).map(|(_, v)| *v).sum::<usize>();
        if sum > *m as usize {
            return Ok(false);
        }
    }
    for (r, cap) in ctx.caps.iter().enumerate() {
        let taken: usize = cells.iter().zip(x.iter()).filter(|(c, _)| c.1 == r).map(|(_, v)| *v).sum();
        if taken > *cap {
            return Ok(false);
        }
    }
    for (j, (_, n, _)) in ctx.ex.iter().enumerate() {
        let have: usize = base[j] + cells.iter().zip(x.iter()).enumerate().filter(|(_, (c, _))| c.0 >> j & 1 == 1).map(|(idx, (c, v))| if idx < i { *v } else { c.2.max(*v) }).sum::<usize>();
        if have < *n as usize {
            return Ok(false);
        }
    }
    if i == cells.len() {
        return Ok(ctx.ex.iter().enumerate().all(|(j, (_, n, _))| base[j] + cells.iter().zip(x.iter()).filter(|(c, _)| c.0 >> j & 1 == 1).map(|(_, v)| *v).sum::<usize>() >= *n as usize));
    }
    // fewer values first: they can only help the maxima, and more are tried only when a minimum needs them
    for take in 0..=cells[i].2 {
        x[i] = take;
        if go(i + 1, x, base, ctx, budget)? {
            return Ok(true);
        }
    }
    x[i] = 0;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(n: i64) -> Literal {
        Literal { lexical: n.to_string(), datatype: Datatype::Integer }
    }

    fn dec(s: &str) -> Literal {
        Literal { lexical: s.into(), datatype: Datatype::Decimal }
    }

    fn string(s: &str) -> Literal {
        Literal { lexical: s.into(), datatype: Datatype::String }
    }

    fn ge(n: i64) -> DataRange {
        DataRange::Facets(Datatype::Integer, vec![(Facet::MinInclusive, int(n))])
    }

    fn lt(n: i64) -> DataRange {
        DataRange::Facets(Datatype::Integer, vec![(Facet::MaxExclusive, int(n))])
    }

    fn ty(t: Datatype) -> DataRange {
        DataRange::Datatype(t)
    }

    #[test]
    fn a_value_needs_a_range_that_has_one() {
        assert!(feasible(&[], &[], &[(1, &ty(Datatype::Integer))], &[]).unwrap());
        let empty = DataRange::And(vec![ty(Datatype::Integer), ty(Datatype::String)]);
        assert!(!feasible(&[], &[], &[(1, &empty)], &[]).unwrap(), "no value is both an integer and a string");
        assert!(!feasible(&[], &[&ge(18)], &[(1, &lt(18))], &[]).unwrap(), "all values >= 18 but one < 18 wanted");
        assert!(feasible(&[], &[&ge(18)], &[(1, &ge(20))], &[]).unwrap());
    }

    #[test]
    fn integers_are_discrete_and_decimals_are_dense() {
        let between = DataRange::And(vec![DataRange::Facets(Datatype::Integer, vec![(Facet::MinExclusive, int(1)), (Facet::MaxExclusive, int(3))])]);
        assert!(feasible(&[], &[], &[(1, &between)], &[]).unwrap(), "2 lies between 1 and 3");
        assert!(!feasible(&[], &[], &[(2, &between)], &[]).unwrap(), "but there is only one such integer");
        let dense = DataRange::Facets(Datatype::Decimal, vec![(Facet::MinExclusive, dec("1")), (Facet::MaxExclusive, dec("2"))]);
        assert!(feasible(&[], &[], &[(50, &dense)], &[]).unwrap(), "between 1 and 2 there are as many decimals as you like");
        let next = DataRange::Facets(Datatype::Integer, vec![(Facet::MinExclusive, int(1)), (Facet::MaxExclusive, int(2))]);
        assert!(!feasible(&[], &[], &[(1, &next)], &[]).unwrap(), "no integer strictly between 1 and 2");
    }

    #[test]
    fn cardinalities_count_values_in_ranges() {
        let any = ty(Datatype::String);
        assert!(!feasible(&[], &[], &[(2, &any)], &[(1, &any)]).unwrap(), "at least 2 and at most 1 strings");
        assert!(feasible(&[], &[], &[(2, &any)], &[(2, &any)]).unwrap());
        // two strings wanted, at most one of them in {"a"}: fine, there are other strings
        let a = DataRange::OneOf(vec![string("a")]);
        assert!(feasible(&[], &[], &[(2, &any)], &[(1, &a)]).unwrap());
        // but if the strings are restricted to {"a"} only one value exists
        assert!(!feasible(&[], &[&a], &[(2, &any)], &[]).unwrap());
        // overlapping ranges share their values
        let small = DataRange::Facets(Datatype::Integer, vec![(Facet::MaxInclusive, int(5))]);
        let big = ge(3);
        assert!(!feasible(&[], &[&DataRange::Facets(Datatype::Integer, vec![(Facet::MinInclusive, int(3)), (Facet::MaxInclusive, int(4))])], &[(3, &small)], &[]).unwrap());
        assert!(feasible(&[], &[], &[(3, &small), (3, &big)], &[(4, &DataRange::Datatype(Datatype::Integer))]).unwrap());
    }

    #[test]
    fn asserted_values_count_and_must_fit() {
        assert!(!feasible(&[int(1), int(2)], &[], &[], &[(1, &DataRange::Top)]).unwrap(), "two different values, at most one allowed");
        assert!(feasible(&[int(1), Literal { lexical: "01".into(), datatype: Datatype::Integer }], &[], &[], &[(1, &DataRange::Top)]).unwrap(), "1 and 01 are the same value");
        assert!(!feasible(&[int(1)], &[&ge(2)], &[], &[]).unwrap(), "1 is not in [2, inf)");
        assert!(feasible(&[int(7)], &[&ge(2)], &[(1, &ge(5))], &[]).unwrap());
    }

    #[test]
    fn the_value_spaces_are_separate() {
        let d = ty(Datatype::Decimal);
        let f = ty(Datatype::Double);
        assert!(!feasible(&[], &[&d], &[(1, &f)], &[]).unwrap(), "no number is both a decimal and a double");
        let both = DataRange::Or(vec![d.clone(), f.clone()]);
        assert!(feasible(&[], &[&both], &[(1, &d), (1, &f)], &[(1, &d)]).unwrap());
        // booleans are exactly two values
        let b = ty(Datatype::Boolean);
        assert!(feasible(&[], &[], &[(2, &b)], &[]).unwrap());
        assert!(!feasible(&[], &[], &[(3, &b)], &[]).unwrap());
    }

    #[test]
    fn integer_subtypes_have_their_bounds() {
        assert!(feasible(&[], &[&ty(Datatype::NonNegativeInteger)], &[(1, &ty(Datatype::PositiveInteger))], &[]).unwrap());
        assert!(!feasible(&[], &[&ty(Datatype::NonNegativeInteger)], &[(1, &ty(Datatype::NegativeInteger))], &[]).unwrap());
        assert!(feasible(&[], &[&ty(Datatype::Byte)], &[(256, &ty(Datatype::Short))], &[]).unwrap());
        assert!(!feasible(&[], &[&ty(Datatype::Byte)], &[(257, &ty(Datatype::Short))], &[]).unwrap(), "a byte has only 256 values");
    }

    #[test]
    fn unreadable_literals_are_an_error_not_a_guess() {
        let bad = Literal { lexical: "abc".into(), datatype: Datatype::Integer };
        assert!(matches!(feasible(&[bad], &[], &[], &[]), Err(DataError::Unsupported(_))));
    }
}
