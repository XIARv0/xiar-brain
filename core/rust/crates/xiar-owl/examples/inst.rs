// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Developer tool: is `<individual>` an instance of `<class>` in the ontology? (the entailment our reasoner answers)
//! usage: inst <file.ofn> <individual IRI> <class IRI>
use xiar_owl::model::*;
use xiar_owl::reasoner::Reasoner;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let o = xiar_owl::functional::parse(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let r = Reasoner::new(&o).with_budget(2_000_000, 200_000);
    println!("{:?}", r.instance_of(&Iri::new(a[2].clone()), &ClassExpr::class(&a[3])));
}
