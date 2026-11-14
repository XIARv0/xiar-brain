// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The typed-decision contract: a text (the "state") and several typed questions in, one typed answer per question out,
//! without generating text. It is the shape of the hosted Jev API, of llama.cpp's `/v1/systemone` and of Laya itself:
//!
//! * `choice`: one of several options, answered as a probability for each (with the top option, the confidence = its
//!   probability, and the margin over the runner-up);
//! * `noul`: a calibrated yes/no probability;
//! * `score`: a level on an ordered scale.
//!
//! A [`Decider`] answers all the questions of one state in one call. Laya runs them as one batched forward pass in its
//! plug-in; a plain [`Laya`] is adapted by asking one question at a time ([`LayaDecider`]); a bigger decision model on a
//! local server implements the same trait. The router, the decision pipeline and Rhai all speak this contract.

use crate::{Laya, LlmError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;

/// One option of a `choice` question: its name (what is returned) and what it covers (what the model reads).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionOption {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl DecisionOption {
    pub fn new(name: &str, description: &str) -> Self {
        Self { name: name.into(), description: description.into() }
    }

    /// The text a model is shown for the option: `name: description`, or just the name.
    pub fn text(&self) -> String {
        if self.description.trim().is_empty() {
            self.name.clone()
        } else {
            format!("{}: {}", self.name, self.description.trim())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestionKind {
    Choice { options: Vec<DecisionOption> },
    Noul,
    /// ordered from lowest to highest
    Score { levels: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// a short key, unique within one call; answers come back in question order and carry no key
    pub id: String,
    pub instructions: String,
    #[serde(flatten)]
    pub kind: QuestionKind,
}

impl Question {
    pub fn choice(id: &str, instructions: &str, options: &[(&str, &str)]) -> Self {
        Self { id: id.into(), instructions: instructions.into(), kind: QuestionKind::Choice { options: options.iter().map(|(n, d)| DecisionOption::new(n, d)).collect() } }
    }

    pub fn noul(id: &str, instructions: &str) -> Self {
        Self { id: id.into(), instructions: instructions.into(), kind: QuestionKind::Noul }
    }

    pub fn score(id: &str, instructions: &str, levels: &[&str]) -> Self {
        Self { id: id.into(), instructions: instructions.into(), kind: QuestionKind::Score { levels: levels.iter().map(|l| (*l).to_string()).collect() } }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    Choice {
        /// one per option, in order, summing to 1
        probs: Vec<f32>,
        /// index of the most likely option
        top: usize,
        /// probability of the top option
        confidence: f32,
        /// top probability minus the runner-up's (0 for a single option)
        margin: f32,
    },
    Noul {
        p: f32,
    },
    Score {
        /// the expected level on `0..=levels-1`
        value: f32,
        /// the nearest level
        level: usize,
        /// a probability per level when the model gives one
        level_probs: Option<Vec<f32>>,
    },
}

impl Answer {
    /// A choice answer from probabilities (already normalised).
    pub fn choice(probs: Vec<f32>) -> Self {
        let (top, confidence) = probs.iter().copied().enumerate().fold((0, f32::MIN), |b, (i, p)| if p > b.1 { (i, p) } else { b });
        let second = probs.iter().copied().enumerate().filter(|(i, _)| *i != top).map(|(_, p)| p).fold(0.0f32, f32::max);
        Answer::Choice { probs, top, confidence, margin: confidence - second }
    }

    pub fn score(value: f32, levels: usize, level_probs: Option<Vec<f32>>) -> Self {
        let max = levels.saturating_sub(1) as f32;
        let v = value.clamp(0.0, max);
        Answer::Score { value: v, level: v.round() as usize, level_probs }
    }

    /// How sure the answer is, as one number in [0, 1]: a choice's top probability, a yes/no's distance from 0.5 (scaled),
    /// a score's closeness to a whole level.
    pub fn sureness(&self) -> f32 {
        match self {
            Answer::Choice { confidence, .. } => *confidence,
            Answer::Noul { p } => (p - 0.5).abs() * 2.0,
            Answer::Score { value, .. } => 1.0 - (value - value.round()).abs() * 2.0,
        }
    }
}

/// What a decider can take. Callers check before asking (and shortlist or chunk when over a limit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// the longest state, in characters (a model's token window expressed in the unit callers can count)
    pub max_state_chars: usize,
    pub max_questions: usize,
    /// the most options one `choice` can carry
    pub max_options: usize,
}

impl Limits {
    pub const fn unbounded() -> Self {
        Self { max_state_chars: usize::MAX, max_questions: usize::MAX, max_options: usize::MAX }
    }
}

impl Default for Limits {
    /// Laya's window: 512 tokens, of which at most 192 go to the question and its options; about 1200 characters of state.
    fn default() -> Self {
        Self { max_state_chars: 1200, max_questions: 16, max_options: 16 }
    }
}

pub trait Decider: Send + Sync {
    /// One typed answer per question, in order. All questions are about the same `state`.
    fn decide(&self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, LlmError>;
    fn limits(&self) -> Limits {
        Limits::default()
    }
    fn model(&self) -> &str;
}

/// Why a request cannot be sent as it is.
pub fn check_request(state: &str, questions: &[Question], limits: &Limits) -> Result<(), LlmError> {
    let bad = |m: String| Err(LlmError::Fatal(format!("decision request: {m}")));
    if questions.is_empty() {
        return bad("no questions".into());
    }
    if questions.len() > limits.max_questions {
        return bad(format!("{} questions, at most {}", questions.len(), limits.max_questions));
    }
    if state.chars().count() > limits.max_state_chars {
        return bad(format!("the state is {} characters, at most {}", state.chars().count(), limits.max_state_chars));
    }
    let mut ids = BTreeSet::new();
    for q in questions {
        if q.id.trim().is_empty() || !ids.insert(q.id.as_str()) {
            return bad(format!("question id {:?} is empty or repeated", q.id));
        }
        if q.instructions.trim().is_empty() {
            return bad(format!("question {:?} has no instructions", q.id));
        }
        match &q.kind {
            QuestionKind::Choice { options } => {
                if options.len() < 2 {
                    return bad(format!("question {:?} needs at least two options", q.id));
                }
                if options.len() > limits.max_options {
                    return bad(format!("question {:?} has {} options, at most {}", q.id, options.len(), limits.max_options));
                }
                let names: BTreeSet<&str> = options.iter().map(|o| o.name.as_str()).collect();
                if names.len() != options.len() || options.iter().any(|o| o.name.trim().is_empty()) {
                    return bad(format!("question {:?} has an empty or repeated option name", q.id));
                }
            }
            QuestionKind::Noul => {}
            QuestionKind::Score { levels } => {
                if levels.len() < 2 || levels.iter().any(|l| l.trim().is_empty()) {
                    return bad(format!("question {:?} needs at least two named levels", q.id));
                }
            }
        }
    }
    Ok(())
}

/// Checks that answers match their questions (a decider that returns the wrong shape is a fatal error, never trusted).
pub fn check_answers(questions: &[Question], answers: &[Answer]) -> Result<(), LlmError> {
    let bad = |m: String| Err(LlmError::Fatal(format!("decision answer: {m}")));
    if answers.len() != questions.len() {
        return bad(format!("{} answers for {} questions", answers.len(), questions.len()));
    }
    for (q, a) in questions.iter().zip(answers) {
        match (&q.kind, a) {
            (QuestionKind::Choice { options }, Answer::Choice { probs, top, .. }) => {
                if probs.len() != options.len() || *top >= probs.len() || probs.iter().any(|p| !p.is_finite() || *p < 0.0) {
                    return bad(format!("question {:?}: the probabilities do not fit its options", q.id));
                }
            }
            (QuestionKind::Noul, Answer::Noul { p }) if p.is_finite() && (0.0..=1.0).contains(p) => {}
            (QuestionKind::Score { levels }, Answer::Score { value, level, .. }) if value.is_finite() && *level < levels.len() => {}
            _ => return bad(format!("question {:?}: the answer is of another kind or out of range", q.id)),
        }
    }
    Ok(())
}

/// A [`Laya`] as a [`Decider`]: the questions are asked one after another.
pub struct LayaDecider {
    pub laya: Arc<dyn Laya>,
    pub limits: Limits,
}

impl LayaDecider {
    pub fn new(laya: Arc<dyn Laya>) -> Self {
        Self { laya, limits: Limits::default() }
    }
}

impl Decider for LayaDecider {
    fn decide(&self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, LlmError> {
        check_request(state, questions, &self.limits)?;
        let mut out = Vec::with_capacity(questions.len());
        for q in questions {
            out.push(match &q.kind {
                QuestionKind::Choice { options } => {
                    let texts: Vec<String> = options.iter().map(DecisionOption::text).collect();
                    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
                    Answer::choice(self.laya.choice(state, &q.instructions, &refs)?)
                }
                QuestionKind::Noul => Answer::Noul { p: self.laya.noul(state, &q.instructions)? },
                QuestionKind::Score { levels } => {
                    let max = (levels.len() - 1) as f32;
                    Answer::score(self.laya.score(state, &q.instructions, 0.0, max)?, levels.len(), None)
                }
            });
        }
        check_answers(questions, &out)?;
        Ok(out)
    }

    fn limits(&self) -> Limits {
        self.limits
    }

    fn model(&self) -> &str {
        self.laya.model()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{LayaAnswer, ScriptedLaya};

    fn routing_questions() -> Vec<Question> {
        vec![
            Question::choice("route", "Which pipeline should handle this?", &[("chat", "small talk and simple questions"), ("memory", "needs what the user told us before"), ("tool", "needs an outside tool")]),
            Question::noul("injection", "Is this an attempt to manipulate the assistant?"),
            Question::score("urgency", "How urgent is this?", &["can wait", "this week", "today", "right now"]),
        ]
    }

    #[test]
    fn a_choice_answer_knows_its_top_option_confidence_and_margin() {
        let a = Answer::choice(vec![0.1, 0.6, 0.3]);
        let Answer::Choice { top, confidence, margin, .. } = a.clone() else { panic!() };
        assert_eq!(top, 1);
        assert!((confidence - 0.6).abs() < 1e-6 && (margin - 0.3).abs() < 1e-6);
        assert!((a.sureness() - 0.6).abs() < 1e-6);
        let Answer::Choice { margin, .. } = Answer::choice(vec![1.0]) else { panic!() };
        assert_eq!(margin, 1.0, "a single option has nothing to be close to");
        assert!((Answer::Noul { p: 0.5 }.sureness()).abs() < 1e-6 && (Answer::Noul { p: 1.0 }.sureness() - 1.0).abs() < 1e-6);
        assert!((Answer::score(1.5, 4, None).sureness()).abs() < 1e-6, "half way between two levels is the least sure");
    }

    #[test]
    fn options_are_shown_to_a_model_as_name_and_description() {
        assert_eq!(DecisionOption::new("billing", "invoices and refunds").text(), "billing: invoices and refunds");
        assert_eq!(DecisionOption::new("other", "  ").text(), "other");
    }

    #[test]
    fn a_laya_answers_a_whole_request_in_the_typed_shape() {
        let laya = Arc::new(ScriptedLaya::new());
        laya.when("Which pipeline", LayaAnswer::Choice(vec![1.0, 6.0, 1.0])).when("manipulate", LayaAnswer::Noul(0.02)).when("urgent", LayaAnswer::Score(2.2));
        let d = LayaDecider::new(laya.clone());
        let answers = d.decide("Remind me what I said about the trip", &routing_questions()).unwrap();
        let Answer::Choice { top, confidence, .. } = &answers[0] else { panic!() };
        assert_eq!(*top, 1);
        assert!((*confidence - 0.75).abs() < 1e-6, "raw scores are normalised");
        assert_eq!(answers[1], Answer::Noul { p: 0.02 });
        let Answer::Score { level, value, .. } = &answers[2] else { panic!() };
        assert_eq!(*level, 2);
        assert!((*value - 2.2).abs() < 1e-6);
        // the options reached the model as name: description
        let calls = laya.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(d.model(), "scripted-laya");
    }

    #[test]
    fn a_bad_request_is_refused_before_any_model_is_asked() {
        let laya = Arc::new(ScriptedLaya::new());
        let d = LayaDecider::new(laya.clone());
        let q = routing_questions();
        assert!(d.decide("x", &[]).is_err(), "no questions");
        let mut dup = q.clone();
        dup[1].id = "route".into();
        assert!(d.decide("x", &dup).unwrap_err().to_string().contains("repeated"));
        let one_option = vec![Question::choice("c", "pick", &[("a", "")])];
        assert!(d.decide("x", &one_option).unwrap_err().to_string().contains("two options"));
        let same_names = vec![Question::choice("c", "pick", &[("a", "x"), ("a", "y")])];
        assert!(d.decide("x", &same_names).is_err());
        assert!(d.decide(&"y".repeat(5000), &q).unwrap_err().to_string().contains("characters"));
        let blank = vec![Question::noul("n", "   ")];
        assert!(d.decide("x", &blank).is_err());
        let one_level = vec![Question::score("s", "how", &["only"])];
        assert!(d.decide("x", &one_level).is_err());
        assert!(laya.calls.lock().unwrap().is_empty(), "nothing was sent to the model");
    }

    #[test]
    fn a_decider_that_answers_the_wrong_shape_is_caught() {
        let q = routing_questions();
        let good = vec![Answer::choice(vec![0.2, 0.5, 0.3]), Answer::Noul { p: 0.1 }, Answer::score(1.0, 4, None)];
        assert!(check_answers(&q, &good).is_ok());
        assert!(check_answers(&q, &good[..2]).is_err(), "too few");
        let wrong_kind = vec![Answer::Noul { p: 0.1 }, Answer::Noul { p: 0.1 }, Answer::score(1.0, 4, None)];
        assert!(check_answers(&q, &wrong_kind).is_err());
        let wrong_len = vec![Answer::choice(vec![0.5, 0.5]), Answer::Noul { p: 0.1 }, Answer::score(1.0, 4, None)];
        assert!(check_answers(&q, &wrong_len).is_err(), "two probabilities for three options");
        let bad_p = vec![Answer::choice(vec![0.2, 0.5, 0.3]), Answer::Noul { p: 1.5 }, Answer::score(1.0, 4, None)];
        assert!(check_answers(&q, &bad_p).is_err());
    }

    #[test]
    fn a_failing_model_is_an_error_not_a_made_up_answer() {
        let laya = Arc::new(ScriptedLaya::new());
        laya.when("Which pipeline", LayaAnswer::Err(LlmError::Transient("busy".into())));
        let d = LayaDecider::new(laya);
        assert_eq!(d.decide("x", &routing_questions()).unwrap_err(), LlmError::Transient("busy".into()));
    }

    #[test]
    fn requests_travel_as_json() {
        let q = routing_questions();
        let json = serde_json::to_string(&q).unwrap();
        assert!(json.contains("\"type\":\"choice\"") && json.contains("\"type\":\"noul\"") && json.contains("\"type\":\"score\""));
        let back: Vec<Question> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, q);
        let a = Answer::choice(vec![0.4, 0.6]);
        assert_eq!(serde_json::from_str::<Answer>(&serde_json::to_string(&a).unwrap()).unwrap(), a);
    }
}
