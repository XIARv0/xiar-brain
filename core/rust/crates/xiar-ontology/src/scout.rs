// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The scout: asks a model for candidate classes and relations in a document. Port of the legacy `OntologyScout`: the text
//! is cut to 2000 characters, the model is asked twice (classes, then relations among those classes), a malformed reply
//! keeps whatever was read before the first bad item, and a failed call just yields nothing.

use crate::critic::json_object;
use crate::models::*;
use std::collections::BTreeMap;
use std::sync::Arc;
use xiar_llm::{ChatMessage, ChatRequest, LlmEngine};
use xiar_prompts::PromptStore;

const MAX_TEXT_CHARS: usize = 2000;

pub struct Scout {
    engine: Arc<dyn LlmEngine>,
    prompts: Arc<PromptStore>,
}

fn confidence(item: &serde_json::Value) -> Option<f64> {
    match item.get("confidence") {
        None | Some(serde_json::Value::Null) => Some(0.5),
        Some(v) => v.as_f64().filter(|c| (0.0..=1.0).contains(c)),
    }
}

fn text_of(item: &serde_json::Value, key: &str) -> String {
    item.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

pub fn parse_classes(reply: &str, document_id: &str) -> Vec<CandidateClass> {
    let mut out = Vec::new();
    let Some(v) = json_object(reply) else { return out };
    for item in v.get("classes").and_then(|c| c.as_array()).into_iter().flatten() {
        // a bad item ends the list, as the legacy parser's exception did
        let (Some(name), Some(conf)) = (item.get("name").and_then(|n| n.as_str()), confidence(item)) else { break };
        out.push(CandidateClass { name: name.to_string(), description: text_of(item, "description"), source_document: document_id.to_string(), confidence: conf, occurrences: 1 });
    }
    out
}

pub fn parse_relations(reply: &str, document_id: &str) -> Vec<CandidateRelation> {
    let mut out = Vec::new();
    let Some(v) = json_object(reply) else { return out };
    for item in v.get("relations").and_then(|c| c.as_array()).into_iter().flatten() {
        let field = |k: &str| item.get(k).and_then(|n| n.as_str());
        let (Some(s), Some(p), Some(o), Some(conf)) = (field("subject"), field("predicate"), field("object"), confidence(item)) else { break };
        out.push(CandidateRelation { subject_class: s.to_string(), predicate: p.to_string(), object_class: o.to_string(), description: text_of(item, "description"), source_document: document_id.to_string(), confidence: conf, occurrences: 1 });
    }
    out
}

impl Scout {
    pub fn new(engine: Arc<dyn LlmEngine>, prompts: Arc<PromptStore>) -> Self {
        Self { engine, prompts }
    }

    fn ask(&self, prompt_name: &str, vars: &[(&str, &str)]) -> Option<String> {
        let prompt = self.prompts.render(prompt_name, vars).ok()?;
        let req = ChatRequest { messages: vec![ChatMessage::user(&prompt)], temperature: Some(0.3), ..Default::default() };
        self.engine.chat(&req).ok()?.message.content
    }

    pub fn scout_document(&self, document_text: &str, document_id: &str, dataset_name: &str) -> OntologyCandidate {
        let text: String = document_text.chars().take(MAX_TEXT_CHARS).collect();
        let classes = self.ask("ontology/scout_classes", &[("text", &text)]).map(|r| parse_classes(&r, document_id)).unwrap_or_default();
        let relations = if classes.is_empty() {
            Vec::new()
        } else {
            let names = classes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ");
            self.ask("ontology/scout_relations", &[("text", &text), ("classes", &names)]).map(|r| parse_relations(&r, document_id)).unwrap_or_default()
        };
        let mut properties = BTreeMap::new();
        properties.insert("document_id".to_string(), serde_json::json!(document_id));
        properties.insert("dataset_name".to_string(), serde_json::json!(dataset_name));
        properties.insert("text_length".to_string(), serde_json::json!(document_text.chars().count()));
        OntologyCandidate { classes, relations, properties }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiar_llm::testing::ScriptedEngine;

    fn scout(replies: &[&str]) -> (Scout, Arc<ScriptedEngine>) {
        let e = Arc::new(ScriptedEngine::new("scout", 8192));
        for r in replies {
            e.push(ChatMessage::assistant(Some(r), vec![]));
        }
        (Scout::new(e.clone(), Arc::new(PromptStore::new(None, crate::BUNDLED))), e)
    }

    #[test]
    fn a_document_yields_classes_then_relations_with_provenance() {
        let (s, e) = scout(&[
            "{\"classes\": [{\"name\": \"Agent\", \"description\": \"AI entity\", \"confidence\": 0.9}, {\"name\": \"Task\"}]}",
            "Here you go: {\"relations\": [{\"subject\": \"Agent\", \"predicate\": \"uses\", \"object\": \"Task\", \"description\": \"runs it\", \"confidence\": 0.8}]}",
        ]);
        let c = s.scout_document("Agents run tasks.", "doc-1", "XIAR");
        assert_eq!(c.classes.len(), 2);
        assert_eq!((c.classes[0].name.as_str(), c.classes[0].confidence, c.classes[0].source_document.as_str()), ("Agent", 0.9, "doc-1"));
        assert_eq!(c.classes[1].confidence, 0.5, "a missing confidence defaults to 0.5");
        assert_eq!((c.relations[0].predicate.as_str(), c.relations[0].confidence), ("uses", 0.8));
        assert_eq!(c.properties["text_length"], 17);
        let first = e.last_request().unwrap();
        assert_eq!(first.temperature, Some(0.3));
        let sent = first.messages[0].content.clone().unwrap();
        assert!(sent.contains("CONCEPTS: Agent, Task") && sent.contains("Agents run tasks."), "{sent}");
    }

    #[test]
    fn the_text_is_cut_to_two_thousand_characters() {
        let (s, e) = scout(&["{\"classes\": []}"]);
        s.scout_document(&"§".repeat(5000), "d", "XIAR");
        let sent = e.last_request().unwrap().messages[0].content.clone().unwrap();
        assert_eq!(sent.matches('§').count(), 2000);
    }

    #[test]
    fn a_bad_item_keeps_what_came_before_it() {
        let cs = parse_classes("{\"classes\": [{\"name\": \"A\", \"confidence\": 0.4}, {\"description\": \"no name\"}, {\"name\": \"C\"}]}", "d");
        assert_eq!(cs.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["A"]);
        let cs = parse_classes("{\"classes\": [{\"name\": \"A\", \"confidence\": 1.7}, {\"name\": \"B\"}]}", "d");
        assert!(cs.is_empty(), "a confidence outside 0..1 is invalid");
        assert!(parse_classes("not json", "d").is_empty() && parse_classes("{\"classes\": 5}", "d").is_empty());
        let rs = parse_relations("{\"relations\": [{\"subject\": \"A\", \"predicate\": \"p\", \"object\": \"B\"}, {\"subject\": \"A\"}]}", "d");
        assert_eq!(rs.len(), 1);
    }

    #[test]
    fn failures_and_empty_results_cost_nothing_more_than_an_empty_candidate_set() {
        let down = Arc::new(ScriptedEngine::new("scout", 8192));
        down.push_err(xiar_llm::LlmError::Fatal("offline".into()));
        let s = Scout::new(down.clone(), Arc::new(PromptStore::new(None, crate::BUNDLED)));
        let c = s.scout_document("text", "d", "XIAR");
        assert!(c.classes.is_empty() && c.relations.is_empty());
        assert_eq!(down.call_count(), 1, "with no classes the relation question is never asked");
        let (s, e) = scout(&["{\"classes\": [{\"name\": \"A\"}]}"]);
        let c = s.scout_document("text", "d", "XIAR");
        assert_eq!((c.classes.len(), c.relations.len()), (1, 0), "a failed relation call keeps the classes");
        assert_eq!(e.call_count(), 2);
    }
}
