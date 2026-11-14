// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Test doubles. `ScriptedEngine` plays back canned responses and records every request; `HashEmbedder` produces
//! deterministic bag-of-words vectors so embedding search can be tested without a model.

use crate::*;
use std::collections::VecDeque;
use std::sync::Mutex;

pub struct ScriptedEngine {
    model: String,
    window: u32,
    script: Mutex<VecDeque<Result<ChatMessage, LlmError>>>,
    pub requests: Mutex<Vec<ChatRequest>>,
    /// tokens reported per call (prompt, completion)
    pub usage_per_call: Mutex<(u32, u32)>,
}

impl ScriptedEngine {
    pub fn new(model: &str, window: u32) -> Self {
        Self { model: model.into(), window, script: Mutex::new(VecDeque::new()), requests: Mutex::new(vec![]), usage_per_call: Mutex::new((10, 5)) }
    }

    pub fn push(&self, msg: ChatMessage) -> &Self {
        self.script.lock().unwrap().push_back(Ok(msg));
        self
    }

    pub fn push_err(&self, e: LlmError) -> &Self {
        self.script.lock().unwrap().push_back(Err(e));
        self
    }

    /// An assistant turn that calls tools: `calls = [(name, json_args)]`.
    pub fn push_calls(&self, thought: Option<&str>, calls: &[(&str, &str)]) -> &Self {
        let n = self.requests.lock().unwrap().len() + self.script.lock().unwrap().len();
        let tcs = calls.iter().enumerate().map(|(i, (name, args))| ToolCall { id: format!("call_{n}_{i}"), name: name.to_string(), arguments: args.to_string() }).collect();
        self.push(ChatMessage::assistant(thought, tcs))
    }

    pub fn remaining(&self) -> usize {
        self.script.lock().unwrap().len()
    }

    pub fn call_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub fn last_request(&self) -> Option<ChatRequest> {
        self.requests.lock().unwrap().last().cloned()
    }
}

impl LlmEngine for ScriptedEngine {
    fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        self.requests.lock().unwrap().push(req.clone());
        let next = self.script.lock().unwrap().pop_front();
        match next {
            Some(Ok(message)) => {
                let (p, c) = *self.usage_per_call.lock().unwrap();
                Ok(ChatResponse { message, usage: Usage { prompt_tokens: p, completion_tokens: c, total_tokens: p + c } })
            }
            Some(Err(e)) => Err(e),
            None => Err(LlmError::Fatal("ScriptedEngine: script exhausted (the agent made more LLM calls than the test planned)".into())),
        }
    }
    fn context_window(&self) -> u32 {
        self.window
    }
    fn model(&self) -> &str {
        &self.model
    }
}

/// Hashing-trick embedder: each lowercase alphanumeric token adds weight to bucket `hash(token) % dim`.
pub struct HashEmbedder {
    dim: u32,
}

impl HashEmbedder {
    pub fn new(dim: u32) -> Self {
        Self { dim }
    }
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl EmbedEngine for HashEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, LlmError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim as usize];
                for tok in t.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|x| x.len() > 1) {
                    v[(fnv(tok) % u64::from(self.dim)) as usize] += 1.0;
                }
                if v.iter().all(|x| *x == 0.0) {
                    v[0] = 1.0; // never return the zero vector (cosine is undefined)
                }
                v
            })
            .collect())
    }
    fn dim(&self) -> u32 {
        self.dim
    }
    fn model(&self) -> &str {
        "hash-embedder"
    }
    fn plugin(&self) -> &str {
        "test-hash"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_engine_replays_in_order_and_records_requests() {
        let e = ScriptedEngine::new("m", 1000);
        e.push_calls(Some("thinking"), &[("send_message", "{\"message\":\"hi\"}")]).push(ChatMessage::assistant(Some("done"), vec![]));
        let req = ChatRequest { messages: vec![ChatMessage::user("hello")], ..Default::default() };
        let r1 = e.chat(&req).unwrap();
        assert_eq!(r1.message.tool_calls[0].name, "send_message");
        assert_eq!(r1.usage.total_tokens, 15);
        assert_eq!(e.chat(&req).unwrap().message.content.as_deref(), Some("done"));
        assert!(matches!(e.chat(&req), Err(LlmError::Fatal(_))), "exhausted script is a loud error");
        assert_eq!(e.call_count(), 3);
        assert_eq!(e.last_request().unwrap().messages[0].content.as_deref(), Some("hello"));
    }

    #[test]
    fn hash_embedder_is_deterministic_and_similarity_tracks_shared_words() {
        let h = HashEmbedder::new(256);
        let v = h.embed(&["dark mode theme".into(), "dark mode theme".into(), "dog walks park".into(), "".into()]).unwrap();
        assert_eq!(v[0], v[1]);
        let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / (a.iter().map(|x| x * x).sum::<f32>().sqrt() * b.iter().map(|x| x * x).sum::<f32>().sqrt());
        assert!(cos(&v[0], &v[1]) > 0.99);
        assert!(cos(&v[0], &v[2]) < 0.2);
        assert!(v[3].iter().any(|x| *x != 0.0), "empty text still yields a non-zero vector");
        assert_eq!(h.dim(), 256);
    }

    #[test]
    fn token_estimates_are_monotonic() {
        assert_eq!(estimate_tokens(""), 0);
        assert!(estimate_tokens("abcdefgh") < estimate_tokens("abcdefghijklmnop"));
        let mut u = Usage::default();
        u += Usage { prompt_tokens: 1, completion_tokens: 2, total_tokens: 3 };
        u += Usage { prompt_tokens: 1, completion_tokens: 2, total_tokens: 3 };
        assert_eq!(u.total_tokens, 6);
    }
}

/// A canned answer for [`ScriptedLaya`].
#[derive(Debug, Clone)]
pub enum LayaAnswer {
    Noul(f32),
    Choice(Vec<f32>),
    Score(f32),
    Err(LlmError),
}

/// A Laya double: rules match on a substring of the query (and optionally of the text); the first match wins. Every call
/// is recorded as `(kind, query, text)`.
#[derive(Default)]
pub struct ScriptedLaya {
    rules: Mutex<Vec<(String, Option<String>, LayaAnswer)>>,
    pub calls: Mutex<Vec<(String, String, String)>>,
}

impl ScriptedLaya {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn when(&self, query_contains: &str, answer: LayaAnswer) -> &Self {
        self.rules.lock().unwrap().push((query_contains.to_string(), None, answer));
        self
    }

    pub fn when_text(&self, query_contains: &str, text_contains: &str, answer: LayaAnswer) -> &Self {
        self.rules.lock().unwrap().push((query_contains.to_string(), Some(text_contains.to_string()), answer));
        self
    }

    fn find(&self, kind: &str, text: &str, query: &str) -> Result<LayaAnswer, LlmError> {
        self.calls.lock().unwrap().push((kind.to_string(), query.to_string(), text.to_string()));
        let rules = self.rules.lock().unwrap();
        rules
            .iter()
            .find(|(q, t, _)| query.contains(q.as_str()) && t.as_ref().is_none_or(|t| text.contains(t.as_str())))
            .map(|(_, _, a)| a.clone())
            .ok_or_else(|| LlmError::Fatal(format!("ScriptedLaya: no rule for {kind} query {query:?}")))
    }
}

impl Laya for ScriptedLaya {
    fn choice(&self, text: &str, query: &str, options: &[&str]) -> Result<Vec<f32>, LlmError> {
        match self.find("choice", text, query)? {
            LayaAnswer::Choice(p) => crate::laya::normalize_choice(p, options.len()),
            LayaAnswer::Err(e) => Err(e),
            other => Err(LlmError::Fatal(format!("ScriptedLaya: rule for {query:?} is not a choice: {other:?}"))),
        }
    }
    fn noul(&self, text: &str, query: &str) -> Result<f32, LlmError> {
        match self.find("noul", text, query)? {
            LayaAnswer::Noul(p) => crate::laya::check_probability(p),
            LayaAnswer::Err(e) => Err(e),
            other => Err(LlmError::Fatal(format!("ScriptedLaya: rule for {query:?} is not a noul: {other:?}"))),
        }
    }
    fn score(&self, text: &str, query: &str, lo: f32, hi: f32) -> Result<f32, LlmError> {
        match self.find("score", text, query)? {
            LayaAnswer::Score(v) if v.is_finite() => Ok(v.clamp(lo, hi)),
            LayaAnswer::Err(e) => Err(e),
            other => Err(LlmError::Fatal(format!("ScriptedLaya: rule for {query:?} is not a score: {other:?}"))),
        }
    }
    fn model(&self) -> &str {
        "scripted-laya"
    }
}

