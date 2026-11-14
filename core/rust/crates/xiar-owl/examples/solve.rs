// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Developer tool: classify one functional-syntax ontology and print the verdicts and timing.
use xiar_owl::functional::parse;
use xiar_owl::reasoner::Reasoner;

fn main() {
    let path = std::env::args().nth(1).expect("usage: solve <file.ofn>");
    let o = parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let r = Reasoner::new(&o).with_budget(env("XIAR_STEPS", 100_000), env("XIAR_NODES", 3000));
    let t = std::time::Instant::now();
    println!("consistent: {:?} ({:?}) steps/nodes/depth {:?}", r.is_consistent(), t.elapsed(), r.last_run.get());
    if std::env::var_os("XIAR_ONLY_CONSISTENT").is_some() {
        return;
    }
    if std::env::var_os("XIAR_PHASES").is_some() {
        if std::env::var_os("XIAR_NEGSAT").is_some() {
            for c in o.class_names() {
                let t = std::time::Instant::now();
                let v = r.is_satisfiable(&xiar_owl::model::ClassExpr::Not(Box::new(xiar_owl::model::ClassExpr::Class(c.clone()))));
                if t.elapsed().as_millis() > 200 || v.is_err() {
                    println!("not {}: {:?} ({:?}) {:?}", c.local(), v, t.elapsed(), r.last_run.get());
                }
            }
            println!("negsat done");
            return;
        }
        let t = std::time::Instant::now();
        let k = r.classify();
        println!("classify: {:?} ({:?}) steps/nodes/depth {:?}", k.as_ref().map(|k| k.all_supers.len()), t.elapsed(), r.last_run.get());
        let all = std::env::var_os("XIAR_ALLTYPES").is_some();
        for (n, i) in o.individual_names().into_iter().enumerate().take(if all { usize::MAX } else { 6 }) {
            let t = std::time::Instant::now();
            let v = r.types(&i).map(|s| s.len());
            if !all || t.elapsed().as_millis() > 500 || v.is_err() {
                println!("types #{n} {}: {:?} ({:?}) {:?}", i.local(), v, t.elapsed(), r.last_run.get());
            }
        }
        return;
    }
    let only: Option<Vec<String>> = std::env::var("XIAR_CLASS").ok().map(|v| v.split(',').map(str::to_string).collect());
    for c in o.class_names() {
        if only.as_ref().is_some_and(|n| !n.iter().any(|x| x == c.local())) {
            continue;
        }
        let t = std::time::Instant::now();
        let v = r.is_satisfiable(&xiar_owl::model::ClassExpr::Class(c.clone()));
        println!("sat {}: {:?} ({:?}) steps/nodes/depth {:?}", c.local(), v, t.elapsed(), r.last_run.get());
    }
}
