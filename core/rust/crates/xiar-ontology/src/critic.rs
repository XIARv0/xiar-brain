// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The critic: rule-based structural review plus an optional model review for suggestions. The score decides promotion.
//! Port of the legacy `OntologyCritic` (rules and scoring identical; cycle detection explores in sorted order so the
//! number of warnings does not depend on hash order).

use crate::engineer::parts;
use crate::models::ValidationResult;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use xiar_llm::{ChatMessage, ChatRequest, LlmEngine};
use xiar_owl::model::Ontology;
use xiar_prompts::PromptStore;

fn camel_case(name: &str) -> bool {
    let mut c = name.chars();
    c.next().is_some_and(|f| f.is_ascii_uppercase()) && c.all(|x| x.is_ascii_alphanumeric())
}

/// `(errors, warnings)` from the structure alone.
pub fn structural_review(o: &Ontology) -> (Vec<String>, Vec<String>, usize, usize) {
    let (classes, triples) = parts(o);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if classes.is_empty() {
        errors.push("Ontology must contain at least one class".to_string());
    }
    for c in &classes {
        if !camel_case(c) {
            errors.push(format!("Class '{c}' does not follow CamelCase naming convention"));
        }
    }
    for (d, p, r) in &triples {
        if p.is_empty() {
            errors.push("Found relation with empty predicate".to_string());
        }
        if !d.is_empty() && !classes.contains(d) {
            warnings.push(format!("Relation '{p}' references non-existent domain class '{d}'"));
        }
        if !r.is_empty() && !classes.contains(r) {
            warnings.push(format!("Relation '{p}' references non-existent range class '{r}'"));
        }
    }
    // cycles in the domain -> range graph
    // roots are tried in the order the properties appear (alphabetical by property name, as the engineer writes them),
    // which is what the legacy code's dictionary order gave; neighbours are explored in sorted order
    let mut graph: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut roots: Vec<&str> = Vec::new();
    let mut by_property: Vec<&(String, String, String)> = triples.iter().collect();
    by_property.sort_by(|a, b| (&a.1, &a.0, &a.2).cmp(&(&b.1, &b.0, &b.2)));
    for (d, _, r) in by_property {
        if !d.is_empty() && !r.is_empty() {
            if !graph.contains_key(d.as_str()) {
                roots.push(d);
            }
            graph.entry(d).or_default().insert(r);
        }
    }
    fn has_cycle<'a>(n: &'a str, g: &BTreeMap<&'a str, BTreeSet<&'a str>>, visited: &mut BTreeSet<&'a str>, stack: &mut BTreeSet<&'a str>) -> bool {
        visited.insert(n);
        stack.insert(n);
        for &m in g.get(n).into_iter().flatten() {
            if !visited.contains(m) {
                if has_cycle(m, g, visited, stack) {
                    return true;
                }
            } else if stack.contains(m) {
                return true;
            }
        }
        stack.remove(n);
        false
    }
    let mut visited = BTreeSet::new();
    for &n in &roots {
        if !visited.contains(n) && has_cycle(n, &graph, &mut visited, &mut BTreeSet::new()) {
            warnings.push(format!("Circular dependency detected involving class '{n}'"));
        }
    }
    for (d, p, r) in &triples {
        if d.is_empty() || r.is_empty() {
            errors.push(format!("Relation '{p}' must have both domain and range defined"));
        }
    }
    (errors, warnings, classes.len(), triples.len())
}

/// Score in [0, 1]: -0.2 per error, -0.05 per warning, +0.1 for 5 or more classes, +0.1 for 3 or more relations.
pub fn quality_score(errors: usize, warnings: usize, classes: usize, relations: usize) -> f64 {
    let mut s = 1.0 - errors as f64 * 0.2 - warnings as f64 * 0.05;
    if classes >= 5 {
        s += 0.1;
    }
    if relations >= 3 {
        s += 0.1;
    }
    s.clamp(0.0, 1.0)
}

pub struct Critic {
    engine: Option<Arc<dyn LlmEngine>>,
    prompts: Arc<PromptStore>,
}

impl Critic {
    pub fn new(engine: Option<Arc<dyn LlmEngine>>, prompts: Arc<PromptStore>) -> Self {
        Self { engine, prompts }
    }

    pub fn validate(&self, o: &Ontology) -> ValidationResult {
        let (errors, warnings, classes, relations) = structural_review(o);
        let suggestions = self.semantic_review(o).unwrap_or_default();
        ValidationResult { is_valid: errors.is_empty(), quality_score: quality_score(errors.len(), warnings.len(), classes, relations), errors, warnings, suggestions, class_count: classes, relation_count: relations }
    }

    /// Model review for suggestions only; any failure just means no suggestions.
    fn semantic_review(&self, o: &Ontology) -> Option<Vec<String>> {
        let engine = self.engine.as_ref()?;
        let (classes, triples) = parts(o);
        let relations: String = triples.iter().map(|(d, p, r)| format!("  - {}--[{}]--> {}", if d.is_empty() { "?" } else { d }, p, if r.is_empty() { "?" } else { r })).collect::<Vec<_>>().join("\n");
        let classes = classes.iter().cloned().collect::<Vec<_>>().join(", ");
        let prompt = self.prompts.render("ontology/critic_review", &[("classes", &classes), ("relations", &relations)]).ok()?;
        let req = ChatRequest { messages: vec![ChatMessage::user(&prompt)], temperature: Some(0.3), ..Default::default() };
        let reply = engine.chat(&req).ok()?.message.content?;
        parse_suggestions(&reply)
    }
}

/// The text from the first `{` to the last `}`, parsed as JSON.
pub fn json_object(reply: &str) -> Option<serde_json::Value> {
    let (a, b) = (reply.find('{')?, reply.rfind('}')?);
    (a < b).then(|| serde_json::from_str(&reply[a..=b]).ok()).flatten()
}

pub fn parse_suggestions(reply: &str) -> Option<Vec<String>> {
    let v = json_object(reply)?;
    let list = v.get("suggestions")?.as_array()?;
    Some(list.iter().filter_map(|s| match s {
        serde_json::Value::String(t) if !t.is_empty() => Some(t.clone()),
        serde_json::Value::Null | serde_json::Value::Bool(false) => None,
        serde_json::Value::String(_) => None,
        other => Some(other.to_string()),
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineer::build;
    use xiar_llm::testing::ScriptedEngine;

    fn onto(classes: &[&str], rels: &[(&str, &str, &str)]) -> Ontology {
        build(&classes.iter().map(|s| s.to_string()).collect(), &rels.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect())
    }

    fn critic(engine: Option<Arc<dyn LlmEngine>>) -> Critic {
        Critic::new(engine, Arc::new(PromptStore::new(None, crate::BUNDLED)))
    }

    #[test]
    fn a_clean_ontology_scores_full_marks() {
        let r = critic(None).validate(&onto(&["Agent", "Task", "Tool", "Plan", "Goal"], &[("Agent", "uses", "Tool"), ("Agent", "has", "Goal"), ("Plan", "serves", "Goal")]));
        assert!(r.is_valid && r.errors.is_empty() && r.warnings.is_empty());
        assert_eq!(r.quality_score, 1.0, "1.0 + 0.1 + 0.1 clamps to 1.0");
        assert_eq!((r.class_count, r.relation_count), (5, 3));
    }

    #[test]
    fn structural_problems_are_errors_and_cost_points() {
        let o = onto(&[], &[]);
        assert_eq!(structural_review(&o).0, ["Ontology must contain at least one class"]);
        let o = onto(&["agent", "Task2", "Bad_Name"], &[]);
        let e = structural_review(&o).0;
        assert_eq!(e, ["Class 'Bad_Name' does not follow CamelCase naming convention", "Class 'agent' does not follow CamelCase naming convention"]);
        let mut o = onto(&["Agent"], &[("Agent", "uses", "Agent")]);
        o.axioms.push(xiar_owl::model::Axiom::DeclareObjectProperty(xiar_owl::model::Iri::new(format!("{}orphan", crate::models::NS))));
        let (e, w, _, _) = structural_review(&o);
        assert_eq!(e, ["Relation 'orphan' must have both domain and range defined"]);
        assert_eq!(w, ["Circular dependency detected involving class 'Agent'"]);
    }

    #[test]
    fn references_to_missing_classes_are_warnings() {
        let o = onto(&["Agent"], &[("Agent", "uses", "Ghost"), ("Spirit", "haunts", "Agent")]);
        let w = structural_review(&o).1;
        assert!(w.contains(&"Relation 'uses' references non-existent range class 'Ghost'".to_string()));
        assert!(w.contains(&"Relation 'haunts' references non-existent domain class 'Spirit'".to_string()));
    }

    #[test]
    fn the_score_follows_the_legacy_formula() {
        assert_eq!(quality_score(0, 0, 0, 0), 1.0);
        assert!((quality_score(1, 2, 0, 0) - 0.7).abs() < 1e-9);
        assert_eq!(quality_score(10, 0, 0, 0), 0.0);
        assert!((quality_score(2, 0, 5, 3) - 0.8).abs() < 1e-9, "1 - 0.4 + 0.1 + 0.1");
    }

    #[test]
    fn the_model_review_adds_suggestions_and_never_blocks() {
        let e = Arc::new(ScriptedEngine::new("critic", 8192));
        e.push(ChatMessage::assistant(Some("sure ```{\"suggestions\": [\"add an inverse for uses\", \"\", \"link Agent and Task\"]}``` done"), vec![]));
        let r = critic(Some(e.clone())).validate(&onto(&["Agent", "Task"], &[("Agent", "uses", "Task")]));
        assert_eq!(r.suggestions, ["add an inverse for uses", "link Agent and Task"]);
        let sent = e.last_request().unwrap().messages[0].content.clone().unwrap();
        assert!(sent.contains("CLASSES: Agent, Task") && sent.contains("Agent--[uses]--> Task"), "{sent}");
        let down = Arc::new(ScriptedEngine::new("critic", 8192));
        down.push_err(xiar_llm::LlmError::Fatal("offline".into()));
        let r = critic(Some(down)).validate(&onto(&["Agent"], &[]));
        assert!(r.suggestions.is_empty() && r.is_valid, "a failing review changes nothing else");
        assert!(critic(None).validate(&onto(&["Agent"], &[])).suggestions.is_empty());
    }

    #[test]
    fn json_is_found_inside_chatter() {
        assert_eq!(parse_suggestions("no json here"), None);
        assert_eq!(parse_suggestions("x {\"suggestions\": \"not a list\"} y"), None);
        assert_eq!(parse_suggestions("{\"suggestions\": [1, \"a\"]}"), Some(vec!["1".into(), "a".into()]));
    }
}
