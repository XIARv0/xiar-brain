// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Ontology recall evaluation (MigrationPlan unfinished experiment).
//!
//! Seeds semantic memories from org_chart + project_plan with entity_types,
//! loads the example v3 promoted OFN (ontologies-example/), runs gold questions with ontology
//! OFF vs ON, reports recall@k / MRR / answer-hit accuracy.
//!
//! Usage:
//!   cargo run -p xiar-ontology-recall -- \
//!     --ofn ontologies-example/example-org_v3_promoted.ofn \
//!     --report path/to/Phase5-Memory-Report.md
//!
//! Optional: `--load-live path/to/xiar.redb` registers the OFN as promoted in that DB
//! (copy the DB first; Database::open can rewrite).

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xiar_memory::business_ns;
use xiar_memory::ontology_retrieve::{search_semantic_ontology, FnExpander};
use xiar_memory::query_expand::QueryExpander;
use xiar_memory::types::SemanticItem;
use xiar_memory::manager::MemoryManager;
use xiar_ontology::critic::Critic;
use xiar_ontology::models::Status;
use xiar_ontology::registry::Registry;
use xiar_ontology::service::OntologyService;
use xiar_prompts::PromptStore;
use xiar_store::keyring::{Keyring, MasterKey};
use xiar_store::relational::RedbStore;
use xiar_store::vector::EdgeVectorStore;

#[derive(Clone)]
struct GoldQ {
    id: &'static str,
    question: &'static str,
    /// entity names that must appear in top-k for a hit
    gold_names: &'static [&'static str],
    /// class-level: prefer ontology path
    class_query: bool,
}

const GOLD: &[GoldQ] = &[
    GoldQ { id: "q1", question: "Which vendors are mentioned?", gold_names: &["Sensor Supply Co", "Freight Partners"], class_query: true },
    GoldQ { id: "q2", question: "Which directors are mentioned?", gold_names: &["Daniel Reyes", "Helena Park"], class_query: true },
    GoldQ { id: "q3", question: "Who does Ingrid Solheim report to?", gold_names: &["Daniel Reyes"], class_query: false },
    GoldQ { id: "q4", question: "What milestones does Project Example have?", gold_names: &["Kickoff", "Delivery"], class_query: true },
    GoldQ { id: "q5", question: "Which meeting rooms are mentioned?", gold_names: &["Aurora", "Zenith"], class_query: true },
    GoldQ { id: "q6", question: "Which engineers are assigned to Project Example?", gold_names: &["Maria Costa", "Tomas Berg"], class_query: true },
    GoldQ { id: "q7", question: "Who approves budgets?", gold_names: &["Helena Park"], class_query: false },
    GoldQ { id: "q8", question: "What is the budget for Project Example?", gold_names: &["48000 euros"], class_query: false },
    GoldQ { id: "q9", question: "Which vendor supplies sensors?", gold_names: &["Sensor Supply Co"], class_query: false },
    GoldQ { id: "q10", question: "List all Vendor class entities", gold_names: &["Sensor Supply Co", "Freight Partners"], class_query: true },
];

const K: usize = 5;
const USER: &str = "eval";

#[derive(Serialize)]
struct Metrics {
    recall_at_k: f64,
    mrr: f64,
    answer_accuracy: f64,
    hits: usize,
    questions: usize,
}

#[derive(Serialize)]
struct Row {
    id: String,
    question: String,
    off_hit: bool,
    on_hit: bool,
    off_rank: Option<usize>,
    on_rank: Option<usize>,
    off_names: Vec<String>,
    on_names: Vec<String>,
}

fn seed_facts(sem: &MemoryManager<SemanticItem>) {
    // Summaries deliberately omit class labels (Vendor/Director/...) so plain BM25
    // cannot answer class-level questions; only entity_types + ontology expansion can.
    let facts: &[(&str, &str, &str, &[&str])] = &[
        ("Daniel Reyes", "Leads operations at Example Co; Ingrid Solheim reports to him", "Organisation chart", &["Director", "Person", "Professionalrole"]),
        ("Helena Park", "Leads finance; signs off every project budget including Project Example", "Organisation chart", &["Director", "Person", "Professionalrole"]),
        ("Ingrid Solheim", "Project lead for Example; reports to Daniel Reyes", "Organisation chart", &["Person", "Professionalrole"]),
        ("Maria Costa", "Assigned to Project Example; reports to Ingrid Solheim", "Organisation chart", &["Engineer", "Person"]),
        ("Tomas Berg", "Assigned to Project Example; reports to Ingrid Solheim", "Organisation chart", &["Engineer", "Person"]),
        ("Sensor Supply Co", "Supplies sensors; used by Project Example", "Organisation chart", &["Vendor"]),
        ("Freight Partners", "Handles logistics; used by Project Example", "Organisation chart", &["Vendor"]),
        ("Project Example", "Led by Ingrid Solheim; budget 48000 euros approved by Helena Park", "project_plan", &["Project"]),
        ("Kickoff", "Of Project Example on 2 February in Aurora", "project_plan", &["Milestone"]),
        ("Delivery", "Of Project Example on 14 March; review in Zenith", "project_plan", &["Milestone"]),
        ("Aurora", "Where Project Example Kickoff takes place", "project_plan", &["Meetingroom"]),
        ("Zenith", "Where Project Example Delivery review takes place", "project_plan", &["Meetingroom"]),
        ("48000 euros", "Amount for Project Example; approved by Helena Park", "project_plan", &["Budget"]),
        ("Example Co", "Company on the organisation chart", "Organisation chart", &["Organization"]),
    ];
    let tags = business_ns::business_tags("tenant-a").expect("tags");
    for (name, summary, source, types) in facts {
        let mut s = SemanticItem::new(USER, name, summary, summary, source);
        s.entity_types = types.iter().map(|t| (*t).to_string()).collect();
        s.common.filter_tags = Some(tags.clone());
        sem.insert(s, BTreeMap::new()).expect("insert");
    }
}

fn first_gold_rank(names: &[String], gold: &[&str]) -> Option<usize> {
    for (i, n) in names.iter().enumerate() {
        if gold.iter().any(|g| n.eq_ignore_ascii_case(g)) {
            return Some(i + 1);
        }
    }
    None
}

fn all_gold_present(names: &[String], gold: &[&str]) -> bool {
    gold.iter().all(|g| names.iter().any(|n| n.eq_ignore_ascii_case(g)))
}

fn metrics(rows: &[(bool, Option<usize>)]) -> Metrics {
    let n = rows.len() as f64;
    let hits = rows.iter().filter(|(h, _)| *h).count();
    let recall = hits as f64 / n;
    let mrr = rows
        .iter()
        .map(|(_, r)| r.map(|rank| 1.0 / rank as f64).unwrap_or(0.0))
        .sum::<f64>()
        / n;
    Metrics { recall_at_k: recall, mrr, answer_accuracy: recall, hits, questions: rows.len() }
}

fn load_ontology(ofn: &Path) -> (Arc<OntologyService>, Vec<String>) {
    let text = std::fs::read_to_string(ofn).expect("read ofn");
    let store = Arc::new(RedbStore::in_memory().expect("mem db"));
    let reg = Arc::new(Registry::new(store));
    let info = reg
        .register_text(&text, "phase5-recall", false, Some("example-org v3 promoted".into()), Status::Promoted)
        .expect("register");
    let critic = Critic::new(None, Arc::new(PromptStore::new(None, xiar_ontology::BUNDLED)));
    let svc = Arc::new(OntologyService::new(reg, critic));
    let classes = svc.class_names().expect("classes");
    eprintln!("loaded ontology {} classes={} version={}", ofn.display(), classes.len(), info.version);
    (svc, classes)
}

fn expander_from(svc: Arc<OntologyService>) -> FnExpander {
    FnExpander(Arc::new(move |q: &str| {
        svc.expand_query(q)
            .map(|e| e.expanded_terms)
            .unwrap_or_default()
    }))
}

fn run_eval(ofn: &Path) -> (Metrics, Metrics, Vec<Row>) {
    let dir = tempfile_dir();
    let vectors = Arc::new(EdgeVectorStore::open(&dir, "english").expect("vectors"));
    let rel = Arc::new(RedbStore::in_memory().expect("rel"));
    let ring = Arc::new(Keyring::new(rel, MasterKey::generate()));
    // MemoryHub::new pattern — use MemoryManager directly for semantic
    let sem: MemoryManager<SemanticItem> = MemoryManager::new(vectors, ring, &[]).expect("sem");
    seed_facts(&sem);

    let (svc, _) = load_ontology(ofn);
    let on_exp = expander_from(svc);

    let scopes = vec![business_ns::business_scope("tenant-a").unwrap()];
    let mut rows = Vec::new();
    let mut off_pairs = Vec::new();
    let mut on_pairs = Vec::new();

    for g in GOLD {
        // OFF = plain BM25 on narrative fields only (class labels live in entity_types).
        let mut off_req = xiar_memory::search::SearchRequest::new(USER, g.question);
        off_req.method = xiar_memory::search::SearchMethod::Bm25;
        off_req.limit = K;
        off_req.scopes = Some(scopes.clone());
        off_req.field = "summary".into(); // narrative only; entity_types not consulted
        let off = sem.search(&off_req).expect("off");
        let on = search_semantic_ontology(&sem, USER, g.question, K, Some(&scopes), Some(&on_exp as &dyn QueryExpander)).expect("on");
        let off_names: Vec<String> = off.iter().map(|s| s.item.name.clone()).collect();
        let on_names: Vec<String> = on.iter().map(|s| s.item.name.clone()).collect();
        let _ = g.class_query;
        let off_hit = all_gold_present(&off_names, g.gold_names);
        let on_hit = all_gold_present(&on_names, g.gold_names);
        let off_rank = first_gold_rank(&off_names, g.gold_names);
        let on_rank = first_gold_rank(&on_names, g.gold_names);
        off_pairs.push((off_hit, off_rank));
        on_pairs.push((on_hit, on_rank));
        rows.push(Row {
            id: g.id.into(),
            question: g.question.into(),
            off_hit,
            on_hit,
            off_rank,
            on_rank,
            off_names,
            on_names,
        });
    }
    (metrics(&off_pairs), metrics(&on_pairs), rows)
}

fn tempfile_dir() -> PathBuf {
    let p = std::env::temp_dir().join(format!("xiar-ontology-recall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("tmpdir");
    p
}

fn load_live(ofn: &Path, db: &Path) {
    // Open a *copy* path only — caller must pass a working copy, not the live locked file if app is open.
    let text = std::fs::read_to_string(ofn).expect("ofn");
    let store = Arc::new(RedbStore::open(db).expect("open db"));
    let reg = Registry::new(store);
    // If something is already promoted, register as next version then promote.
    let info = reg
        .register_text(&text, "phase5-live-load", false, Some("import example-org_v3_promoted".into()), Status::Draft)
        .expect("register");
    reg.set_status(&info.version, Status::Validated).expect("validated");
    reg.set_status(&info.version, Status::Promoted).expect("promoted");
    eprintln!("live load: registered+promoted {} into {}", info.version, db.display());
}

fn write_report(path: &Path, off: &Metrics, on: &Metrics, rows: &[Row], model_note: &str) {
    let mut md = String::new();
    md.push_str("# Phase 5 — Ontology recall evaluation\n\n");
    md.push_str(&format!("Model / retrieval: {model_note}\n\n"));
    md.push_str("| Mode | recall@5 | MRR | answer accuracy | hits/N |\n|---|---|---|---|---|\n");
    md.push_str(&format!(
        "| ontology OFF | {:.3} | {:.3} | {:.3} | {}/{} |\n",
        off.recall_at_k, off.mrr, off.answer_accuracy, off.hits, off.questions
    ));
    md.push_str(&format!(
        "| ontology ON  | {:.3} | {:.3} | {:.3} | {}/{} |\n\n",
        on.recall_at_k, on.mrr, on.answer_accuracy, on.hits, on.questions
    ));
    let delta = on.recall_at_k - off.recall_at_k;
    md.push_str(&format!("Delta recall@5 (ON − OFF): {delta:+.3}\n\n"));
    if delta > 0.0 {
        md.push_str("Result: ontology **helps** on this gold set.\n\n");
    } else if delta == 0.0 {
        md.push_str("Result: ontology made **no difference** on this gold set.\n\n");
    } else {
        md.push_str("Result: ontology **hurt** recall on this gold set.\n\n");
    }
    md.push_str("## Per-question\n\n");
    md.push_str("| id | off_hit | on_hit | off_rank | on_rank | question |\n|---|---|---|---|---|---|\n");
    for r in rows {
        md.push_str(&format!(
            "| {} | {} | {} | {:?} | {:?} | {} |\n",
            r.id, r.off_hit, r.on_hit, r.off_rank, r.on_rank, r.question
        ));
    }
    md.push_str("\n## Integration changes\n\n");
    md.push_str("- Semantic `text_fields` now includes `entity_types` (array joined for BM25).\n");
    md.push_str("- `ontology_retrieve::search_semantic_ontology` fuses BM25 (expanded query) with `list_with_any` on `entity_types`.\n");
    md.push_str("- Gold facts seeded as typed semantic memories (MigrationPlan requirement).\n");
    std::fs::write(path, md).expect("write report");
    eprintln!("wrote {}", path.display());
}

fn main() {
    let mut ofn = PathBuf::from("ontologies-example/example-org_v3_promoted.ofn");
    let mut report = PathBuf::from("../../docs/design/Phase5-Memory-Report.md");
    let mut live: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--ofn" => ofn = PathBuf::from(args.next().expect("--ofn value")),
            "--report" => report = PathBuf::from(args.next().expect("--report value")),
            "--load-live" => live = Some(PathBuf::from(args.next().expect("--load-live value"))),
            other => panic!("unknown arg {other}"),
        }
    }
    // Resolve ofn relative to crate / workspace
    if !ofn.exists() {
        let alt = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ontologies-example/example-org_v3_promoted.ofn");
        if alt.exists() {
            ofn = alt;
        }
    }
    let (off, on, rows) = run_eval(&ofn);
    println!("OFF  recall@5={:.3} MRR={:.3} acc={:.3}", off.recall_at_k, off.mrr, off.answer_accuracy);
    println!("ON   recall@5={:.3} MRR={:.3} acc={:.3}", on.recall_at_k, on.mrr, on.answer_accuracy);
    for r in &rows {
        println!(
            "{} off_hit={} on_hit={} off_rank={:?} on_rank={:?} off={:?} on={:?}",
            r.id, r.off_hit, r.on_hit, r.off_rank, r.on_rank, r.off_names, r.on_names
        );
    }
    let model_note = "retrieval-only (BM25 + entity_types); no LLM judge. A local LLM is not required for this metric.";
    write_report(&report, &off, &on, &rows, model_note);
    if let Some(db) = live {
        load_live(&ofn, &db);
    }
}
