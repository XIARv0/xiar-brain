// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Developer tool: classify every `<n>.ofn` in a directory and write `<n>.ours.json` in the format `tools/owl_compare.py`
//! reads (and `<n>.owl` must already hold the RDF/XML form for the oracle). Files outside the XIAR profile are reported with
//! the reason instead of a verdict. usage: diff_dir <dir> [stem]   (with a stem only <stem>.ofn is done; files that already have a result are skipped)
use std::collections::BTreeSet;
use xiar_owl::functional::parse;
use xiar_owl::model::*;
use xiar_owl::reasoner::{ReasonError, Reasoner};

fn json_list(items: impl Iterator<Item = String>) -> String {
    format!("[{}]", items.map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(","))
}

fn locals(s: &BTreeSet<Iri>) -> BTreeSet<String> {
    s.iter().map(|i| i.0.clone()).collect()
}

fn verdicts(o: &Ontology) -> String {
    let steps: usize = std::env::var("XIAR_OWL_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(2_000_000);
    let cells: usize = std::env::var("XIAR_OWL_CELLS").ok().and_then(|v| v.parse().ok()).unwrap_or(xiar_owl::reasoner::MAX_CELLS);
    let r = Reasoner::new(o).with_budget(steps, 200_000).with_memory_budget(cells);
    match r.is_consistent() {
        Err(ReasonError::Budget(w)) => return format!("{{\"error\":\"budget {w}\"}}"),
        Err(e) => return format!("{{\"error\":\"{e}\"}}"),
        Ok(false) => return "{\"consistent\":false}".to_string(),
        Ok(true) => {}
    }
    // XIAR_PART=classify or =types does only that half (for ontologies too slow to do both in one go); `merge_parts.py` joins them
    let part = std::env::var("XIAR_PART").unwrap_or_default();
    if part == "types" {
        let mut types = Vec::new();
        for i in o.individual_names() {
            match r.types(&i) {
                Ok(t) => types.push(format!("\"{}\":{}", i.0, json_list(locals(&t).into_iter()))),
                Err(e) => return format!("{{\"error\":\"{e}\"}}"),
            }
        }
        return format!("{{\"consistent\":true,\"types\":{{{}}}}}", types.join(","));
    }
    let k = match r.classify() {
        Ok(k) => k,
        Err(e) => return format!("{{\"error\":\"{e}\"}}"),
    };
    let supers = k.all_supers.iter().map(|(c, s)| format!("\"{}\":{}", c.0, json_list(locals(s).into_iter()))).collect::<Vec<_>>().join(",");
    let equiv = k.equivalents.iter().map(|g| json_list(locals(g).into_iter())).collect::<Vec<_>>().join(",");
    let mut types = Vec::new();
    for i in o.individual_names().into_iter().filter(|_| part.is_empty()) {
        match r.types(&i) {
            Ok(t) => types.push(format!("\"{}\":{}", i.0, json_list(locals(&t).into_iter()))),
            Err(e) => return format!("{{\"error\":\"{e}\"}}"),
        }
    }
    format!("{{\"consistent\":true,\"top\":{},\"unsat\":{},\"supers\":{{{}}},\"equiv\":[{}],\"types\":{{{}}}}}", json_list(locals(&k.top_equivalents).into_iter()), json_list(locals(&k.unsatisfiable).into_iter()), supers, equiv, types.join(","))
}


fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: diff_dir <dir>"));
    let mut stats = (0, 0, 0);
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "ofn")).collect();
    files.sort();
    let only = std::env::args().nth(2);
    files.retain(|p| only.as_ref().is_none_or(|s| p.file_stem().is_some_and(|f| f.to_string_lossy() == s.as_str())) && !p.with_extension("ours.json").exists());
    for p in files {
        let text = std::fs::read_to_string(&p).unwrap();
        let out = match parse(&text) {
            Ok(o) => {
                stats.0 += 1;
                verdicts(&o)
            }
            Err(e) => {
                stats.1 += 1;
                let msg = e.to_string().replace('\\', "/").replace('"', "'");
                format!("{{\"error\":\"parse: {msg}\"}}")
            }
        };
        std::fs::write(p.with_extension("ours.json"), out).unwrap();
    }
    let _ = stats.2;
    println!("classified {} ontologies, {} outside the profile", stats.0, stats.1);
}
