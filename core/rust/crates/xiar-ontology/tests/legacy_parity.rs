// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Parity with the legacy Python ontology service: the engineer and critic are deterministic without a model, so the goldens
//! in `golden/legacy.json` (made by `tools/ontology_oracle.py` running the old code) must be reproduced exactly.

use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;
use xiar_ontology::critic::Critic;
use xiar_ontology::engineer::{build, engineer, parts};
use xiar_ontology::models::{CandidateClass, CandidateRelation, OntologyCandidate};
use xiar_prompts::PromptStore;

fn golden() -> Value {
    serde_json::from_str(include_str!("golden/legacy.json")).unwrap()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

fn triples(v: &Value) -> BTreeSet<(String, String, String)> {
    v.as_array().unwrap().iter().map(|t| (t[0].as_str().unwrap().into(), t[1].as_str().unwrap().into(), t[2].as_str().unwrap().into())).collect()
}

#[test]
fn the_engineer_reproduces_the_legacy_service() {
    let g = golden();
    let cases = g["engineer"].as_array().unwrap();
    assert_eq!(cases.len(), 15);
    for c in cases {
        let base = build(&strings(&c["base_classes"]).into_iter().collect(), &triples(&c["base_relations"]));
        let candidates = OntologyCandidate {
            classes: strings(&c["candidate_classes"]).into_iter().map(|name| CandidateClass { name, description: String::new(), source_document: "d".into(), confidence: 0.9, occurrences: 1 }).collect(),
            relations: c["candidate_relations"].as_array().unwrap().iter().map(|r| CandidateRelation { subject_class: r[0].as_str().unwrap().into(), predicate: r[1].as_str().unwrap().into(), object_class: r[2].as_str().unwrap().into(), description: String::new(), source_document: "d".into(), confidence: 0.8, occurrences: 1 }).collect(),
            ..Default::default()
        };
        let (onto, r) = engineer(&candidates, &base, c["version"].as_str().unwrap());
        let (classes, props) = parts(&onto);
        assert_eq!(classes.into_iter().collect::<Vec<_>>(), strings(&c["classes"]), "classes for {}", c["candidate_classes"]);
        assert_eq!(props, triples(&c["properties"]), "relations for {}", c["candidate_classes"]);
        assert_eq!((r.new_version.as_str(), r.classes_added as u64, r.relations_added as u64, r.conflicts_resolved as u64), (c["new_version"].as_str().unwrap(), c["classes_added"].as_u64().unwrap(), c["relations_added"].as_u64().unwrap(), c["conflicts_resolved"].as_u64().unwrap()), "{}", c["candidate_classes"]);
        assert_eq!(r.diff_summary, c["diff_summary"].as_str().unwrap());
    }
}

#[test]
fn the_critic_scores_and_reports_like_the_legacy_service() {
    let critic = Critic::new(None, Arc::new(PromptStore::new(None, xiar_ontology::BUNDLED)));
    for c in golden()["critic"].as_array().unwrap() {
        let onto = build(&strings(&c["classes"]).into_iter().collect(), &triples(&c["properties"]));
        let r = critic.validate(&onto);
        assert_eq!(r.is_valid, c["is_valid"].as_bool().unwrap(), "{c}");
        assert!((r.quality_score - c["quality_score"].as_f64().unwrap()).abs() < 1e-9, "score {} vs {} for {}", r.quality_score, c["quality_score"], c["classes"]);
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        assert_eq!(sorted(r.errors), sorted(strings(&c["errors"])));
        assert_eq!(sorted(r.warnings), sorted(strings(&c["warnings"])));
        assert_eq!((r.class_count as u64, r.relation_count as u64), (c["class_count"].as_u64().unwrap(), c["relation_count"].as_u64().unwrap()));
    }
}
