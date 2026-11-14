// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Developer tool: read a functional-syntax ontology and print it as RDF/XML (the form the Pellet oracle reads).
//! usage: ofn_to_owl <file.ofn> > file.owl
fn main() {
    let path = std::env::args().nth(1).expect("usage: ofn_to_owl <file.ofn>");
    let o = xiar_owl::functional::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    print!("{}", xiar_owl::rdfxml::export(&o).unwrap());
}
