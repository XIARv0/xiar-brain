// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Developer tool: time one subsumption test. usage: subs <file.ofn> <sub local name> <super local name>
use xiar_owl::functional::parse;
use xiar_owl::model::*;
use xiar_owl::reasoner::Reasoner;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let o = parse(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let find = |n: &str| o.class_names().into_iter().find(|c| c.local() == n).unwrap_or_else(|| panic!("no class {n}"));
    let (sub, sup) = (ClassExpr::Class(find(&a[2])), ClassExpr::Class(find(&a[3])));
    let r = Reasoner::new(&o);
    let t = std::time::Instant::now();
    println!("consistent {:?} ({:?})", r.is_consistent(), t.elapsed());
    let t = std::time::Instant::now();
    let v = r.subsumes(&sup, &sub);
    println!("{} <= {}: {:?} ({:?}) steps/nodes/depth {:?}", a[2], a[3], v, t.elapsed(), r.last_run.get());
}
