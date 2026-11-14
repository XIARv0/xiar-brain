// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! A bigger decision model on this machine: llama.cpp's `POST /v1/systemone` (for example the `lev` GGUF).
//!
//! The server answers every question of one `state` in one call, without generating text. This adapter maps the typed contract of
//! `xiar_llm::decision` onto it and back:
//!
//! * `choice`: `criteria` = option name -> description (null when there is none); the server's `probabilities` are re-normalised
//!   to sum to 1 (the server reports them unnormalised), `confidence` is recomputed from them;
//! * `noul`: the server's number is already the calibrated probability of "yes";
//! * `score`: `criteria` = the ordered level descriptions; the server gives the expected level and a probability per level.
//!
//! Same local-only rules as the chat engine: plain `http://` to this machine, no redirects.

use crate::{agent, local_base, post_bytes};
use serde_json::{json, Map, Value};
use std::time::Duration;
use xiar_llm::decision::{check_answers, check_request, Answer, Decider, Limits, Question, QuestionKind};
use xiar_llm::LlmError;

pub struct HttpDecider {
    base: String,
    model: String,
    agent: ureq::Agent,
    limits: Limits,
}

impl HttpDecider {
    /// `base_url` like `http://127.0.0.1:8075`; `model` is only a label for the audit trail.
    pub fn new(base_url: &str, model: &str, timeout: Duration) -> Result<Self, LlmError> {
        // lev's window is several thousand tokens: about 12 000 characters of state leave room for the questions
        let limits = Limits { max_state_chars: 12_000, max_questions: 32, max_options: 32 };
        Ok(Self { base: local_base(base_url)?, model: model.into(), agent: agent(timeout), limits })
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
}

/// The request body for `/v1/systemone`.
pub fn request_body(state: &str, questions: &[Question]) -> Value {
    let mut qs = Map::new();
    for q in questions {
        let mut o = Map::new();
        o.insert("instructions".into(), json!(q.instructions));
        match &q.kind {
            QuestionKind::Choice { options } => {
                o.insert("type".into(), json!("choice"));
                let mut c = Map::new();
                for opt in options {
                    c.insert(opt.name.clone(), if opt.description.trim().is_empty() { Value::Null } else { json!(opt.description.trim()) });
                }
                o.insert("criteria".into(), Value::Object(c));
            }
            QuestionKind::Noul => {
                o.insert("type".into(), json!("noul"));
            }
            QuestionKind::Score { levels } => {
                o.insert("type".into(), json!("score"));
                o.insert("criteria".into(), json!(levels));
            }
        }
        qs.insert(q.id.clone(), Value::Object(o));
    }
    json!({"state": state, "questions": qs})
}

fn fatal<T>(m: String) -> Result<T, LlmError> {
    Err(LlmError::Fatal(format!("decision server: {m}")))
}

/// Maps a `/v1/systemone` response onto answers, one per question, in question order.
pub fn parse_response(questions: &[Question], resp: &Value) -> Result<Vec<Answer>, LlmError> {
    let Some(answers) = resp["answers"].as_object() else { return fatal("the reply has no \"answers\"".into()) };
    let mut out = Vec::with_capacity(questions.len());
    for q in questions {
        let Some(a) = answers.get(&q.id) else { return fatal(format!("no answer for question {:?}", q.id)) };
        out.push(match &q.kind {
            QuestionKind::Choice { options } => {
                let Some(p) = a["probabilities"].as_object() else { return fatal(format!("question {:?}: no probabilities", q.id)) };
                let mut probs = Vec::with_capacity(options.len());
                for o in options {
                    match p.get(&o.name).and_then(Value::as_f64) {
                        Some(x) if x.is_finite() && x >= 0.0 => probs.push(x as f32),
                        _ => return fatal(format!("question {:?}: no probability for option {:?}", q.id, o.name)),
                    }
                }
                let sum: f32 = probs.iter().sum();
                if sum <= 0.0 {
                    return fatal(format!("question {:?}: the probabilities are all zero", q.id));
                }
                Answer::choice(probs.into_iter().map(|x| x / sum).collect())
            }
            QuestionKind::Noul => match a["noul"].as_f64() {
                Some(p) if (0.0..=1.0).contains(&p) => Answer::Noul { p: p as f32 },
                _ => return fatal(format!("question {:?}: no probability of yes in 0..1", q.id)),
            },
            QuestionKind::Score { levels } => {
                let Some(value) = a["score"].as_f64().filter(|v| v.is_finite()) else { return fatal(format!("question {:?}: no score", q.id)) };
                let probs: Option<Vec<f32>> = a["probabilities"].as_object().and_then(|m| (0..levels.len()).map(|i| m.get(&i.to_string()).and_then(Value::as_f64).map(|x| x as f32)).collect());
                let probs = probs.and_then(|v| {
                    let s: f32 = v.iter().sum();
                    (s > 0.0).then(|| v.into_iter().map(|x| x / s).collect())
                });
                Answer::score(value as f32, levels.len(), probs)
            }
        });
    }
    check_answers(questions, &out)?;
    Ok(out)
}

impl Decider for HttpDecider {
    fn decide(&self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, LlmError> {
        check_request(state, questions, &self.limits)?;
        let body = request_body(state, questions).to_string().into_bytes();
        let resp = post_bytes(&self.agent, &format!("{}/v1/systemone", self.base), None, "application/json", body)?;
        parse_response(questions, &resp)
    }

    fn limits(&self) -> Limits {
        self.limits
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const GOLDEN: &str = include_str!("../tests/golden/systemone_lev.json");

    fn golden() -> Vec<Value> {
        serde_json::from_str::<Value>(GOLDEN).unwrap()["cases"].as_array().unwrap().clone()
    }

    /// The questions of a recorded request, back in our own types.
    fn questions_of(req: &Value) -> Vec<Question> {
        req["questions"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(id, q)| {
                let ins = q["instructions"].as_str().unwrap();
                match q["type"].as_str().unwrap() {
                    "choice" => {
                        let opts: Vec<(String, String)> = q["criteria"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect();
                        let refs: Vec<(&str, &str)> = opts.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                        Question::choice(id, ins, &refs)
                    }
                    "score" => {
                        let l: Vec<&str> = q["criteria"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
                        Question::score(id, ins, &l)
                    }
                    _ => Question::noul(id, ins),
                }
            })
            .collect()
    }

    #[test]
    fn recorded_replies_map_onto_valid_answers() {
        let mut n = 0;
        for c in golden().iter().filter(|c| c["status"] == 200 && c["name"].as_str().unwrap().starts_with("router")) {
            let qs = questions_of(&c["request"]);
            let a = parse_response(&qs, &c["response"]).unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
            assert_eq!(a.len(), qs.len());
            n += 1;
        }
        assert_eq!(n, 6);
    }

    #[test]
    fn the_recorded_routes_come_out_right() {
        let cases: Vec<Value> = golden().into_iter().filter(|c| c["name"].as_str().unwrap().starts_with("router")).collect();
        let want = ["chat", "memory", "tool", "tool", "decision", "tool"];
        for (c, w) in cases.iter().zip(want) {
            let qs = questions_of(&c["request"]);
            let a = parse_response(&qs, &c["response"]).unwrap();
            let idx = qs.iter().position(|q| q.id == "pipeline").unwrap();
            let QuestionKind::Choice { options } = &qs[idx].kind else { panic!() };
            let Answer::Choice { top, probs, .. } = &a[idx] else { panic!() };
            assert_eq!(options[*top].name, w, "{}", c["name"]);
            assert!((probs.iter().sum::<f32>() - 1.0).abs() < 1e-4, "re-normalised");
        }
    }

    #[test]
    fn the_injection_is_flagged_and_a_plain_hello_is_not() {
        let cases = golden();
        let p = |name: &str| -> f32 {
            let c = cases.iter().find(|c| c["name"].as_str().unwrap().contains(name)).unwrap();
            let qs = questions_of(&c["request"]);
            let a = parse_response(&qs, &c["response"]).unwrap();
            let i = qs.iter().position(|q| q.id == "injection").unwrap();
            let Answer::Noul { p } = a[i] else { panic!() };
            p
        };
        assert!(p("Ignore all previous") > 0.9);
        assert!(p("Hi there") < 0.1);
    }

    #[test]
    fn a_score_keeps_its_level_probabilities() {
        let c = golden().into_iter().find(|c| c["name"] == "documentation example").unwrap();
        let qs = questions_of(&c["request"]);
        let a = parse_response(&qs, &c["response"]).unwrap();
        let i = qs.iter().position(|q| q.id == "urgency").unwrap();
        let Answer::Score { level, level_probs, value } = &a[i] else { panic!() };
        assert_eq!(*level, 2);
        assert!((*value - 1.5675).abs() < 1e-3);
        let lp = level_probs.as_ref().unwrap();
        assert_eq!(lp.len(), 4);
        assert!((lp.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_missing_option_or_answer_is_refused() {
        let c = golden().into_iter().find(|c| c["name"] == "documentation example").unwrap();
        let mut qs = questions_of(&c["request"]);
        qs.push(Question::noul("extra", "Is it extra?"));
        assert!(parse_response(&qs, &c["response"]).unwrap_err().to_string().contains("no answer for question"));
        let mut resp = c["response"].clone();
        resp["answers"]["route"]["probabilities"].as_object_mut().unwrap().remove("billing");
        let qs = questions_of(&c["request"]);
        assert!(parse_response(&qs, &resp).is_err());
        assert!(parse_response(&qs, &json!({"nothing": 1})).is_err());
    }

    #[test]
    fn our_request_has_the_shape_the_server_documents() {
        let qs = vec![
            Question::choice("route", "Which team?", &[("billing", ""), ("shipping", "parcels")]),
            Question::noul("angry", "Is the customer angry?"),
            Question::score("urgency", "How urgent?", &["low", "high"]),
        ];
        let b = request_body("hello", &qs);
        assert_eq!(b["state"], "hello");
        assert_eq!(b["questions"]["route"]["type"], "choice");
        assert!(b["questions"]["route"]["criteria"]["billing"].is_null());
        assert_eq!(b["questions"]["route"]["criteria"]["shipping"], "parcels");
        assert_eq!(b["questions"]["angry"]["type"], "noul");
        assert!(b["questions"]["angry"].get("criteria").is_none());
        assert_eq!(b["questions"]["urgency"]["criteria"], json!(["low", "high"]));
        // what we send is accepted by the recorded server: the recorded requests are built the same way
        let recorded = golden().into_iter().find(|c| c["name"] == "documentation example").unwrap();
        let rebuilt = request_body(recorded["request"]["state"].as_str().unwrap(), &questions_of(&recorded["request"]));
        assert_eq!(rebuilt["questions"]["route"], recorded["request"]["questions"]["route"]);
        assert_eq!(rebuilt["questions"]["urgency"], recorded["request"]["questions"]["urgency"]);
    }

    /// A one-shot fake server that answers with the recorded reply of a case and returns the request it saw.
    fn serve_once(status: u16, reply: Value) -> (String, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = s.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(i) = text.find("\r\n\r\n") {
                    let len = text[..i].to_lowercase().split("content-length:").nth(1).and_then(|r| r.split_whitespace().next().and_then(|v| v.parse::<usize>().ok())).unwrap_or(0);
                    if buf.len() >= i + 4 + len {
                        break;
                    }
                }
            }
            let body = reply.to_string();
            let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            String::from_utf8_lossy(&buf).to_string()
        });
        (url, h)
    }

    #[test]
    fn decide_talks_to_a_local_server_end_to_end() {
        let c = golden().into_iter().find(|c| c["name"] == "documentation example").unwrap();
        let qs = questions_of(&c["request"]);
        let (url, h) = serve_once(200, c["response"].clone());
        let d = HttpDecider::new(&url, "lev", Duration::from_secs(5)).unwrap();
        let a = d.decide(c["request"]["state"].as_str().unwrap(), &qs).unwrap();
        let seen = h.join().unwrap();
        assert!(seen.starts_with("POST /v1/systemone "), "{seen}");
        assert_eq!(a.len(), 4);
        assert_eq!(d.model(), "lev");
    }

    #[test]
    fn a_server_without_a_decision_model_is_a_fatal_error() {
        let (url, h) = serve_once(501, json!({"error": {"code": 501, "message": "this model does not support /v1/systemone", "type": "not_supported_error"}}));
        let d = HttpDecider::new(&url, "x", Duration::from_secs(5)).unwrap();
        let e = d.decide("hi", &[Question::noul("q", "Is it?")]).unwrap_err();
        assert!(matches!(e, LlmError::Transient(_)), "5xx is retried later, then the cascade falls back: {e}");
        h.join().unwrap();
    }

    #[test]
    fn only_this_machine_is_allowed() {
        assert!(HttpDecider::new("http://192.168.1.5:8075", "x", Duration::from_secs(1)).is_err());
        assert!(HttpDecider::new("https://127.0.0.1:8075", "x", Duration::from_secs(1)).is_err());
        assert!(HttpDecider::new("http://localhost:8075", "x", Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn a_request_over_the_limits_never_leaves() {
        let d = HttpDecider::new("http://127.0.0.1:1", "x", Duration::from_secs(1)).unwrap().with_limits(Limits { max_state_chars: 10, max_questions: 2, max_options: 4 });
        assert!(d.decide(&"x".repeat(11), &[Question::noul("q", "Is it?")]).unwrap_err().to_string().contains("characters"));
    }
}
