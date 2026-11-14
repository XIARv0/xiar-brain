// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! xiar-engine-http — chat and embedding engines for an OpenAI-compatible server running ON THIS MACHINE (llama.cpp's
//! `llama-server`, Ollama, LM Studio, vLLM, mistral.rs server...). This is how a real model is used without any native
//! library in the core: the model runs in the user's own server process.
//!
//! Local-only by construction:
//! * TLS is compiled out of the HTTP client, so only plain `http://` can work;
//! * the host must be a loopback address (`localhost`, `127.0.0.0/8`, `::1`); anything else is refused at construction.
//!   Reaching a remote or cloud provider needs a provider plug-in with a manifest, a named recipient and explicit opt-in;
//! * redirects are disabled, so a local server cannot bounce a request to another host.
//!
//! Wire format: `POST /v1/chat/completions` (messages, tools, forced `tool_choice`, `max_tokens`, `temperature`) and
//! `POST /v1/embeddings`. Failures map onto [`LlmError`]: 429/5xx/timeouts are transient, a context-length complaint is a
//! context overflow (the agent reacts by summarising and retrying), everything else is fatal.

pub mod bearer;
pub mod decide;
pub mod screen;
pub mod voice;

use serde_json::{json, Value};
use std::io::Read;
use std::time::Duration;
use xiar_llm::{ChatMessage, ChatRequest, ChatResponse, EmbedEngine, LlmEngine, LlmError, Role, ToolCall, Usage};

const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HttpEngineConfig {
    /// e.g. `http://127.0.0.1:8080` (a trailing `/v1` is accepted)
    pub base_url: String,
    pub model: String,
    pub context_window: u32,
    /// sent as `Authorization: Bearer ...` when the local server wants one
    pub api_key: Option<String>,
    pub timeout: Duration,
    /// embedding output size (required for `HttpEmbedEngine`)
    pub embedding_dim: Option<u32>,
    /// Ollama only: how many of the model's layers go to the graphics card (`0` = none, the model runs on the processor; a large number =
    /// all). Setting it makes the engine use Ollama's own chat API, which takes this option; its OpenAI-style API does not.
    pub num_gpu: Option<u32>,
}

impl HttpEngineConfig {
    pub fn new(base_url: &str, model: &str, context_window: u32) -> Self {
        Self { base_url: base_url.to_string(), model: model.to_string(), context_window, api_key: None, timeout: Duration::from_secs(300), embedding_dim: None, num_gpu: None }
    }
}

/// Validates the URL and returns the normalized base without a trailing `/v1` or slash.
pub fn local_base(url: &str) -> Result<String, LlmError> {
    let bad = |why: &str| LlmError::Fatal(format!("engine url {url:?}: {why}"));
    let rest = url.strip_prefix("http://").ok_or_else(|| bad("only plain http:// to this machine is supported by the built-in HTTP engine"))?;
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    if authority.contains('@') {
        return Err(bad("credentials in the url are not allowed"));
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split_once(']').map(|(h, _)| h).ok_or_else(|| bad("malformed IPv6 address"))?,
        None => authority.rsplit_once(':').map_or(authority, |(h, port)| if port.chars().all(|c| c.is_ascii_digit()) { h } else { authority }),
    };
    let loopback = host.eq_ignore_ascii_case("localhost") || host == "::1" || host.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback());
    if !loopback {
        return Err(bad("the host must be this machine (localhost / 127.x.x.x / ::1); remote providers need a provider plug-in with explicit opt-in"));
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix("v1").map_or(path, |p| p.trim_end_matches('/'));
    Ok(if path.is_empty() { format!("http://{authority}") } else { format!("http://{authority}/{path}") })
}

/// One request to a program on THIS computer, by port (always `127.0.0.1`): `GET` when `body` is `None`, `POST` of JSON otherwise.
/// `token`, when given, goes in the `X-Xiar-Token` header. Returns the status code and the text of the answer. Used to drive the
/// Unreal Engine character window, which listens only on the loopback address.
pub fn loopback_request(port: u16, path: &str, token: Option<&str>, body: Option<&Value>, timeout: Duration) -> Result<(u16, String), String> {
    if port < 1024 || !path.starts_with('/') || path.contains(['\r', '\n', ' ']) {
        return Err("a loopback request needs a port from 1024 up and a path that starts with /".into());
    }
    let url = format!("http://127.0.0.1:{port}{path}");
    let agent = agent(timeout);
    let mut resp = match body {
        Some(b) => {
            let mut req = agent.post(&url).header("Content-Type", "application/json");
            if let Some(t) = token {
                req = req.header("X-Xiar-Token", t);
            }
            req.send(b.to_string().as_bytes())
        }
        None => {
            let mut req = agent.get(&url);
            if let Some(t) = token {
                req = req.header("X-Xiar-Token", t);
            }
            req.call()
        }
    }
    .map_err(|e| format!("nothing answered on port {port}: {e}"))?;
    let status = resp.status().as_u16();
    let mut text = String::new();
    resp.body_mut().as_reader().take(MAX_RESPONSE_BYTES).read_to_string(&mut text).map_err(|e| format!("could not read the answer: {e}"))?;
    Ok((status, text))
}

/// A file from a program on THIS computer (`127.0.0.1`, a port from 1024 up), at most `max` bytes. For audio and the like that a local engine
/// makes and offers for download.
pub fn loopback_bytes(port: u16, path: &str, timeout: Duration, max: u64) -> Result<Vec<u8>, String> {
    if port < 1024 || !path.starts_with('/') || path.contains(['\r', '\n', ' ']) {
        return Err("a loopback request needs a port from 1024 up and a path that starts with /".into());
    }
    let mut resp = agent(timeout).get(&format!("http://127.0.0.1:{port}{path}")).call().map_err(|e| format!("nothing answered on port {port}: {e}"))?;
    if !(200..300).contains(&resp.status().as_u16()) {
        return Err(format!("the engine answered with code {}", resp.status().as_u16()));
    }
    let mut out = Vec::new();
    resp.body_mut().as_reader().take(max + 1).read_to_end(&mut out).map_err(|e| format!("could not read the file: {e}"))?;
    if out.len() as u64 > max {
        return Err("the file is larger than allowed".into());
    }
    Ok(out)
}

pub(crate) fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).max_redirects(0).http_status_as_error(false).build().into()
}

fn post(agent: &ureq::Agent, url: &str, key: Option<&str>, body: &Value) -> Result<Value, LlmError> {
    post_bytes(agent, url, key, "application/json", body.to_string().into_bytes())
}

/// POST `body` as `content_type` and read a JSON reply; failures are classified like the chat engine's.
pub(crate) fn post_bytes(agent: &ureq::Agent, url: &str, key: Option<&str>, content_type: &str, body: Vec<u8>) -> Result<Value, LlmError> {
    let mut req = agent.post(url).header("Content-Type", content_type);
    if let Some(k) = key {
        req = req.header("Authorization", &format!("Bearer {k}"));
    }
    let mut resp = req.send(&body[..]).map_err(|e| match e {
        ureq::Error::Timeout(_) | ureq::Error::Io(_) | ureq::Error::ConnectionFailed => LlmError::Transient(format!("local engine unreachable: {e}")),
        other => LlmError::Fatal(format!("local engine request failed: {other}")),
    })?;
    let status = resp.status().as_u16();
    let mut text = String::new();
    resp.body_mut().as_reader().take(MAX_RESPONSE_BYTES).read_to_string(&mut text).map_err(|e| LlmError::Transient(format!("could not read the engine response: {e}")))?;
    if (200..300).contains(&status) {
        return serde_json::from_str(&text).map_err(|e| LlmError::Fatal(format!("engine returned invalid JSON: {e}")));
    }
    let detail = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"]["message"].as_str().or_else(|| v["error"].as_str()).map(str::to_string)).unwrap_or_else(|| text.chars().take(300).collect());
    let lower = detail.to_lowercase();
    Err(if status == 429 || status >= 500 {
        LlmError::Transient(format!("HTTP {status}: {detail}"))
    } else if lower.contains("context length") || lower.contains("context_length") || lower.contains("maximum context") || lower.contains("too many tokens") || lower.contains("exceeds the available context") {
        LlmError::ContextOverflow(detail)
    } else {
        LlmError::Fatal(format!("HTTP {status}: {detail}"))
    })
}

fn message_json(m: &ChatMessage) -> Value {
    let role = match m.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut v = json!({"role": role, "content": m.content});
    if !m.tool_calls.is_empty() {
        v["tool_calls"] = m.tool_calls.iter().map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments}})).collect();
    }
    if let Some(id) = &m.tool_call_id {
        v["tool_call_id"] = json!(id);
    }
    if m.role == Role::Tool {
        if let Some(n) = &m.name {
            v["name"] = json!(n);
        }
    }
    v
}

pub fn chat_body(model: &str, req: &ChatRequest) -> Value {
    let mut body = json!({"model": model, "messages": req.messages.iter().map(message_json).collect::<Vec<_>>(), "stream": false});
    if !req.tools.is_empty() {
        body["tools"] = req.tools.iter().map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.parameters}})).collect();
        body["tool_choice"] = match &req.force_tool {
            Some(name) => json!({"type": "function", "function": {"name": name}}),
            None => json!("auto"),
        };
    }
    if let Some(n) = req.max_tokens {
        body["max_tokens"] = json!(n);
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    body
}

fn parse_chat(v: &Value) -> Result<ChatResponse, LlmError> {
    let msg = v["choices"].get(0).map(|c| &c["message"]).filter(|m| m.is_object()).ok_or_else(|| LlmError::Fatal("engine response has no choices[0].message".into()))?;
    let content = match &msg["content"] {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Array(parts) => {
            let t: String = parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("");
            if t.is_empty() { None } else { Some(t) }
        }
        _ => None,
    };
    let tool_calls = msg["tool_calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let name = c["function"]["name"].as_str()?.to_string();
                    let arguments = match &c["function"]["arguments"] {
                        Value::String(s) => s.clone(),
                        Value::Null => "{}".to_string(),
                        other => other.to_string(),
                    };
                    Some(ToolCall { id: c["id"].as_str().filter(|s| !s.is_empty()).map_or_else(|| format!("call-{}", i + 1), str::to_string), name, arguments })
                })
                .collect()
        })
        .unwrap_or_default();
    let u = &v["usage"];
    let n = |k: &str| u[k].as_u64().unwrap_or(0) as u32;
    let (prompt_tokens, completion_tokens) = (n("prompt_tokens"), n("completion_tokens"));
    let total_tokens = u["total_tokens"].as_u64().map_or(prompt_tokens + completion_tokens, |t| t as u32);
    Ok(ChatResponse { message: ChatMessage { role: Role::Assistant, content, name: None, tool_calls, tool_call_id: None }, usage: Usage { prompt_tokens, completion_tokens, total_tokens } })
}

pub struct HttpLlmEngine {
    cfg: HttpEngineConfig,
    base: String,
    agent: ureq::Agent,
}

impl HttpLlmEngine {
    pub fn new(cfg: HttpEngineConfig) -> Result<Self, LlmError> {
        if cfg.context_window == 0 {
            return Err(LlmError::Fatal("context_window must be positive".into()));
        }
        let base = local_base(&cfg.base_url)?;
        let agent = agent(cfg.timeout);
        Ok(Self { cfg, base, agent })
    }
}

/// The body of a request to Ollama's own chat API (`/api/chat`): like the OpenAI-style one, but the options sit in `options` and a tool
/// call's arguments are an object, not text.
pub fn ollama_body(model: &str, req: &ChatRequest, num_gpu: u32, num_ctx: u32) -> Value {
    let msgs: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            let mut v = json!({"role": role, "content": m.content.clone().unwrap_or_default()});
            if !m.tool_calls.is_empty() {
                v["tool_calls"] = m.tool_calls.iter().map(|c| json!({"function": {"name": c.name, "arguments": serde_json::from_str::<Value>(&c.arguments).unwrap_or_else(|_| json!({}))}})).collect();
            }
            if m.role == Role::Tool {
                if let Some(n) = &m.name {
                    v["tool_name"] = json!(n);
                }
            }
            v
        })
        .collect();
    let mut options = json!({"num_gpu": num_gpu, "num_ctx": num_ctx});
    if let Some(n) = req.max_tokens {
        options["num_predict"] = json!(n);
    }
    if let Some(t) = req.temperature {
        options["temperature"] = json!(t);
    }
    let mut body = json!({"model": model, "messages": msgs, "stream": false, "options": options});
    if !req.tools.is_empty() {
        body["tools"] = req.tools.iter().map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.parameters}})).collect();
    }
    body
}

fn parse_ollama(v: &Value) -> Result<ChatResponse, LlmError> {
    let msg = v.get("message").filter(|m| m.is_object()).ok_or_else(|| LlmError::Fatal("Ollama's answer has no message".into()))?;
    let content = msg["content"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let tool_calls: Vec<ToolCall> = msg["tool_calls"]
        .as_array()
        .map(|calls| calls.iter().enumerate().filter_map(|(i, c)| Some(ToolCall { id: format!("call-{}", i + 1), name: c["function"]["name"].as_str()?.to_string(), arguments: c["function"]["arguments"].to_string() })).collect())
        .unwrap_or_default();
    let n = |k: &str| v[k].as_u64().unwrap_or(0) as u32;
    let (prompt_tokens, completion_tokens) = (n("prompt_eval_count"), n("eval_count"));
    Ok(ChatResponse { message: ChatMessage { role: Role::Assistant, content, name: None, tool_calls, tool_call_id: None }, usage: Usage { prompt_tokens, completion_tokens, total_tokens: prompt_tokens + completion_tokens } })
}

impl LlmEngine for HttpLlmEngine {
    fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        if let Some(gpu) = self.cfg.num_gpu {
            let v = post(&self.agent, &format!("{}/api/chat", self.base), self.cfg.api_key.as_deref(), &ollama_body(&self.cfg.model, req, gpu, self.cfg.context_window))?;
            return parse_ollama(&v);
        }
        let v = post(&self.agent, &format!("{}/v1/chat/completions", self.base), self.cfg.api_key.as_deref(), &chat_body(&self.cfg.model, req))?;
        parse_chat(&v)
    }
    fn context_window(&self) -> u32 {
        self.cfg.context_window
    }
    fn model(&self) -> &str {
        &self.cfg.model
    }
}

pub struct HttpEmbedEngine {
    cfg: HttpEngineConfig,
    base: String,
    agent: ureq::Agent,
    dim: u32,
}

impl HttpEmbedEngine {
    pub fn new(cfg: HttpEngineConfig) -> Result<Self, LlmError> {
        let dim = cfg.embedding_dim.filter(|d| *d > 0).ok_or_else(|| LlmError::Fatal("embedding_dim must be set for an embedding engine".into()))?;
        let base = local_base(&cfg.base_url)?;
        let agent = agent(cfg.timeout);
        Ok(Self { cfg, base, agent, dim })
    }
}

impl HttpEmbedEngine {
    /// Public model metadata only; never probes by inference or accepts credentials.
    pub fn metadata(base_url: &str) -> Result<Value, LlmError> {
        let base=local_base(base_url)?;
        let mut response=agent(Duration::from_secs(10)).get(&format!("{base}/v1/models")).call()
            .map_err(|_|LlmError::Transient("embedding metadata endpoint unavailable".into()))?;
        let mut text=String::new();
        response.body_mut().as_reader().take(MAX_RESPONSE_BYTES).read_to_string(&mut text)
            .map_err(|_|LlmError::Transient("embedding metadata could not be read".into()))?;
        serde_json::from_str(&text).map_err(|_|LlmError::Fatal("invalid embedding metadata".into()))
    }
    pub fn embed_query(&self, texts:&[String])->Result<Vec<Vec<f32>>,LlmError>{self.embed_kind(texts,"query")}
    fn embed_kind(&self, texts: &[String], kind:&str) -> Result<Vec<Vec<f32>>, LlmError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let v = post(&self.agent, &format!("{}/v1/embeddings", self.base), self.cfg.api_key.as_deref(), &json!({"model": self.cfg.model, "input": texts,"input_type":kind}))?;
        let data = v["data"].as_array().ok_or_else(|| LlmError::Fatal("embedding response has no data array".into()))?;
        let mut rows: Vec<(usize, Vec<f32>)> = Vec::with_capacity(data.len());
        for (pos, d) in data.iter().enumerate() {
            let emb = d["embedding"].as_array().ok_or_else(|| LlmError::Fatal("embedding entry without a vector".into()))?;
            let vec: Vec<f32> = emb.iter().map(|x| x.as_f64().map(|f| f as f32).ok_or_else(|| LlmError::Fatal("non-numeric embedding value".into()))).collect::<Result<_, _>>()?;
            rows.push((d["index"].as_u64().map_or(pos, |i| i as usize), vec));
        }
        rows.sort_by_key(|(i, _)| *i);
        if rows.iter().enumerate().any(|(expected,(index,vector))|expected!=*index||vector.iter().any(|v|!v.is_finite())) {
            return Err(LlmError::Fatal("invalid embedding indexes or non-finite vector".into()));
        }
        if rows.len() != texts.len() {
            return Err(LlmError::Fatal(format!("engine returned {} vectors for {} texts", rows.len(), texts.len())));
        }
        if let Some((_, bad)) = rows.iter().find(|(_, r)| r.len() != self.dim as usize) {
            return Err(LlmError::Fatal(format!("engine returned a {}-d vector, expected {}", bad.len(), self.dim)));
        }
        Ok(rows.into_iter().map(|(_, r)| r).collect())
    }
}
impl EmbedEngine for HttpEmbedEngine {
    fn embed(&self,texts:&[String])->Result<Vec<Vec<f32>>,LlmError>{self.embed_kind(texts,"document")}
    fn dim(&self) -> u32 {
        self.dim
    }
    fn model(&self) -> &str {
        &self.cfg.model
    }
    fn plugin(&self) -> &str {
        "http-local"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use xiar_llm::ToolSpec;

    #[test]
    fn ollama_native_body_carries_the_processor_choice_and_reads_tool_calls() {
        let mut req = ChatRequest { messages: vec![ChatMessage::user("hi"), ChatMessage::assistant(None, vec![ToolCall { id: "c1".into(), name: "f".into(), arguments: "{\"a\":1}".into() }]), ChatMessage::tool_result("f", "c1", "done")], max_tokens: Some(50), temperature: Some(0.1), ..ChatRequest::default() };
        req.tools = vec![ToolSpec { name: "f".into(), description: "d".into(), parameters: json!({"type": "object"}) }];
        let cpu = ollama_body("qwen3:4b", &req, 0, 8192);
        assert_eq!(cpu["options"]["num_gpu"], 0);
        assert_eq!(cpu["options"]["num_predict"], 50);
        assert_eq!(cpu["messages"][1]["tool_calls"][0]["function"]["arguments"]["a"], 1, "arguments are an object for this API");
        assert_eq!(cpu["messages"][2]["tool_name"], "f");
        assert_eq!(ollama_body("m", &req, 999, 4096)["options"]["num_gpu"], 999);
        let r = parse_ollama(&json!({"message": {"role": "assistant", "content": "", "tool_calls": [{"function": {"name": "f", "arguments": {"a": 2}}}]}, "prompt_eval_count": 7, "eval_count": 3})).unwrap();
        assert_eq!((r.message.tool_calls[0].name.as_str(), r.message.tool_calls[0].arguments.as_str(), r.usage.total_tokens), ("f", "{\"a\":2}", 10));
    }

    /// A one-thread HTTP server that answers each request with the next canned `(status, body)` and records
    /// `(path, headers, body)` of every request.
    struct Mock {
        url: String,
        seen: Arc<Mutex<Vec<(String, String, Value)>>>,
        raw: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    fn mock(responses: Vec<(u16, String)>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let raw = Arc::new(Mutex::new(Vec::new()));
        let (s, r) = (seen.clone(), raw.clone());
        std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let (mut len, mut headers) = (0usize, String::new());
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    headers.push_str(&h.to_lowercase());
                }
                let mut buf = vec![0u8; len];
                reader.read_exact(&mut buf).unwrap();
                r.lock().unwrap().push(buf.clone());
                s.lock().unwrap().push((path, headers, serde_json::from_slice(&buf).unwrap_or(Value::Null)));
                let reason = if status == 200 { "OK" } else { "ERR" };
                let resp = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(resp.as_bytes()).unwrap();
            }
        });
        Mock { url, seen, raw }
    }

    #[test]
    fn a_loopback_request_reaches_only_this_computer_and_carries_the_token() {
        let m = mock(vec![(200, r#"{"ok":true}"#.into()), (401, r#"{"ok":false}"#.into())]);
        let port: u16 = m.url.rsplit(':').next().unwrap().parse().unwrap();
        let body = serde_json::json!({"gesture": "Happy"});
        let (status, text) = loopback_request(port, "/segment", Some("s3cret"), Some(&body), Duration::from_secs(5)).unwrap();
        assert_eq!((status, text.as_str()), (200, r#"{"ok":true}"#));
        let (status, _) = loopback_request(port, "/ping", None, None, Duration::from_secs(5)).unwrap();
        assert_eq!(status, 401, "the status is reported, not turned into an error");
        let seen = m.seen.lock().unwrap();
        assert_eq!(seen[0].0, "/segment");
        assert!(seen[0].1.contains("x-xiar-token: s3cret"), "{}", seen[0].1);
        assert_eq!(seen[0].2["gesture"], "Happy");
        drop(seen);
        // a port with nothing on it, and arguments that are not a request, are errors
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(loopback_request(free, "/ping", None, None, Duration::from_millis(500)).is_err());
        assert!(loopback_request(80, "/ping", None, None, Duration::from_secs(1)).is_err());
        assert!(loopback_request(free, "ping", None, None, Duration::from_secs(1)).is_err());
        assert!(loopback_request(free, "/a b", None, None, Duration::from_secs(1)).is_err());
    }

    fn cfg(url: &str) -> HttpEngineConfig {
        let mut c = HttpEngineConfig::new(url, "local-model", 8192);
        c.timeout = Duration::from_secs(10);
        c
    }

    #[test]
    fn only_this_machine_over_plain_http_is_accepted() {
        for ok in ["http://127.0.0.1:8080", "http://localhost:11434", "http://localhost", "http://[::1]:8080/v1", "http://127.0.0.5:1/v1/", "http://LOCALHOST:9"] {
            assert!(local_base(ok).is_ok(), "{ok}");
        }
        for bad in ["https://127.0.0.1:8080", "http://api.openai.com", "http://192.168.1.10:8080", "http://10.0.0.1", "http://localhost.evil.com", "http://user:pw@127.0.0.1", "ftp://localhost", "127.0.0.1:8080", "http://127.0.0.1.nip.io", ""] {
            assert!(local_base(bad).is_err(), "{bad}");
        }
        assert_eq!(local_base("http://127.0.0.1:8080/v1/").unwrap(), "http://127.0.0.1:8080");
        assert_eq!(local_base("http://localhost:1234/api/v1").unwrap(), "http://localhost:1234/api");
    }

    #[test]
    fn chat_request_and_response_roundtrip_with_tools() {
        let reply = json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{"id": "abc", "type": "function", "function": {"name": "send_message", "arguments": "{\"message\":\"hi\"}"}}]}}], "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}});
        let m = mock(vec![(200, reply.to_string())]);
        let mut c = cfg(&m.url);
        c.api_key = Some("local-key".into());
        let e = HttpLlmEngine::new(c).unwrap();
        let req = ChatRequest {
            messages: vec![ChatMessage::system("be brief"), ChatMessage::user("hello"), ChatMessage::assistant(None, vec![ToolCall { id: "t1".into(), name: "lookup".into(), arguments: "{}".into() }]), ChatMessage::tool_result("lookup", "t1", "found")],
            tools: vec![ToolSpec { name: "send_message".into(), description: "send".into(), parameters: json!({"type": "object"}) }],
            max_tokens: Some(256),
            temperature: Some(0.2),
            force_tool: Some("send_message".into()),
        };
        let r = e.chat(&req).unwrap();
        assert_eq!(r.message.content, None);
        assert_eq!((r.message.tool_calls[0].id.as_str(), r.message.tool_calls[0].name.as_str(), r.message.tool_calls[0].arguments.as_str()), ("abc", "send_message", r#"{"message":"hi"}"#));
        assert_eq!(r.usage, Usage { prompt_tokens: 7, completion_tokens: 3, total_tokens: 10 });
        let seen = m.seen.lock().unwrap();
        let (path, headers, body) = &seen[0];
        assert_eq!(path, "/v1/chat/completions");
        assert!(headers.contains("authorization: bearer local-key"));
        assert_eq!(body["model"], "local-model");
        assert_eq!(body["messages"][2]["tool_calls"][0]["function"]["name"], "lookup");
        assert_eq!(body["messages"][3], json!({"role": "tool", "content": "found", "tool_call_id": "t1", "name": "lookup"}));
        assert_eq!(body["tool_choice"], json!({"type": "function", "function": {"name": "send_message"}}));
        assert_eq!((body["max_tokens"].as_u64(), body["stream"].as_bool()), (Some(256), Some(false)));
        assert_eq!(body["tools"][0]["function"]["name"], "send_message");
    }

    #[test]
    fn without_tools_there_is_no_tool_choice_and_plain_text_comes_back() {
        let m = mock(vec![(200, json!({"choices": [{"message": {"content": [{"type": "text", "text": "he"}, {"type": "text", "text": "llo"}]}}]}).to_string())]);
        let e = HttpLlmEngine::new(cfg(&m.url)).unwrap();
        let r = e.chat(&ChatRequest { messages: vec![ChatMessage::user("x")], ..Default::default() }).unwrap();
        assert_eq!(r.message.content.as_deref(), Some("hello"));
        assert_eq!(r.usage, Usage::default(), "a server that reports no usage is fine");
        let body = m.seen.lock().unwrap()[0].2.clone();
        assert!(body.get("tools").is_none() && body.get("tool_choice").is_none() && body.get("max_tokens").is_none());
    }

    #[test]
    fn http_errors_map_to_engine_error_kinds() {
        let overflow = json!({"error": {"message": "This model's maximum context length is 4096 tokens"}}).to_string();
        let m = mock(vec![(429, "slow down".into()), (503, json!({"error": "loading model"}).to_string()), (400, overflow), (400, json!({"error": {"message": "bad request"}}).to_string()), (200, "not json".into()), (200, json!({"choices": []}).to_string())]);
        let e = HttpLlmEngine::new(cfg(&m.url)).unwrap();
        let req = ChatRequest { messages: vec![ChatMessage::user("x")], ..Default::default() };
        assert!(matches!(e.chat(&req), Err(LlmError::Transient(m)) if m.contains("429")));
        assert!(matches!(e.chat(&req), Err(LlmError::Transient(m)) if m.contains("loading model")));
        assert!(matches!(e.chat(&req), Err(LlmError::ContextOverflow(m)) if m.contains("maximum context length")));
        assert!(matches!(e.chat(&req), Err(LlmError::Fatal(m)) if m.contains("bad request")));
        assert!(matches!(e.chat(&req), Err(LlmError::Fatal(m)) if m.contains("invalid JSON")));
        assert!(matches!(e.chat(&req), Err(LlmError::Fatal(m)) if m.contains("no choices")));
    }

    #[test]
    fn a_server_that_is_down_is_a_transient_error_not_a_crash() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port(); // freed again: nothing listens
        let e = HttpLlmEngine::new(cfg(&format!("http://127.0.0.1:{port}"))).unwrap();
        assert!(matches!(e.chat(&ChatRequest { messages: vec![ChatMessage::user("x")], ..Default::default() }), Err(LlmError::Transient(_))));
    }

    #[test]
    fn redirects_are_not_followed() {
        let m = mock(vec![(302, String::new())]);
        let e = HttpLlmEngine::new(cfg(&m.url)).unwrap();
        let r = e.chat(&ChatRequest { messages: vec![ChatMessage::user("x")], ..Default::default() });
        assert!(matches!(&r, Err(LlmError::Fatal(m)) if m.contains("302")), "{r:?}");
    }

    #[test]
    fn embeddings_are_reordered_by_index_and_validated() {
        let ok = json!({"data": [{"index": 1, "embedding": [0.0, 1.0]}, {"index": 0, "embedding": [1.0, 0.0]}]}).to_string();
        let short = json!({"data": [{"index": 0, "embedding": [1.0]}]}).to_string();
        let few = json!({"data": [{"index": 0, "embedding": [1.0, 0.0]}]}).to_string();
        let m = mock(vec![(200, ok), (200, short), (200, few)]);
        let mut c = cfg(&m.url);
        assert!(HttpEmbedEngine::new(c.clone()).is_err(), "the dimension must be configured");
        c.embedding_dim = Some(2);
        let e = HttpEmbedEngine::new(c).unwrap();
        let v = e.embed(&["a".into(), "b".into()]).unwrap();
        assert_eq!(v, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        assert_eq!((m.seen.lock().unwrap()[0].0.as_str(), e.dim(), e.plugin()), ("/v1/embeddings", 2, "http-local"));
        assert!(matches!(e.embed(&["a".into()]), Err(LlmError::Fatal(m)) if m.contains("1-d")));
        assert!(matches!(e.embed(&["a".into(), "b".into()]), Err(LlmError::Fatal(m)) if m.contains("1 vectors for 2 texts")));
        assert!(e.embed(&[]).unwrap().is_empty(), "no request for no texts");
    }

    #[test]
    fn remote_hosts_are_refused_at_construction() {
        assert!(HttpLlmEngine::new(HttpEngineConfig::new("http://api.example.com", "m", 4096)).is_err());
        assert!(HttpLlmEngine::new(HttpEngineConfig::new("http://127.0.0.1:1", "m", 0)).is_err());
    }

    // ------------------------------------------------------------------ the screen monitor's vision and speech steps
    fn frame_2x2() -> xiar_screen::pipeline::Frame {
        xiar_screen::pipeline::Frame { ts_ms: 0, width: 2, height: 2, rgba: vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255], app: "Code".into(), title: "main.rs".into() }
    }

    #[test]
    fn a_screenshot_goes_to_a_local_vision_model_as_a_png_data_url() {
        use crate::screen::HttpDescriber;
        use xiar_screen::pipeline::Describer;
        let m = mock(vec![(200, json!({"choices": [{"message": {"role": "assistant", "content": "  A code editor.  "}}]}).to_string())]);
        let mut c = cfg(&m.url);
        c.model = "qwen2.5vl:3b".into();
        c.api_key = Some("k".into());
        let d = HttpDescriber::new(c, "What is the user doing?").unwrap();
        assert_eq!(d.describe(&frame_2x2()).unwrap(), "A code editor.");
        let seen = m.seen.lock().unwrap();
        let (path, headers, body) = &seen[0];
        assert_eq!(path, "/v1/chat/completions");
        assert!(headers.contains("authorization: bearer k"));
        assert_eq!(body["model"], "qwen2.5vl:3b");
        let parts = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts[0], json!({"type": "text", "text": "What is the user doing?"}));
        let url = parts[1]["image_url"]["url"].as_str().unwrap();
        let b64 = url.strip_prefix("data:image/png;base64,").expect("a PNG data url");
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        // and the picture inside is the one that was sent
        let dec = png::Decoder::new(std::io::Cursor::new(png));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        r.next_frame(&mut buf).unwrap();
        assert_eq!(&buf[..16], &frame_2x2().rgba[..]);
    }

    #[test]
    fn speech_goes_to_a_local_transcription_server_as_a_wav_upload() {
        use crate::screen::HttpTranscriber;
        use xiar_screen::pipeline::{Clip, Transcriber};
        let m = mock(vec![(200, json!({"text": " call mom at five "}).to_string())]);
        let mut c = cfg(&m.url);
        c.model = "large-v3-turbo".into();
        let t = HttpTranscriber::new(c).unwrap();
        let clip = Clip { sample_rate: 16_000, pcm16: vec![0, 100, -100, 32767, -32768] };
        assert_eq!(t.transcribe(&clip).unwrap(), "call mom at five");
        assert_eq!(m.seen.lock().unwrap()[0].0, "/v1/audio/transcriptions");
        assert!(m.seen.lock().unwrap()[0].1.contains("multipart/form-data; boundary="));
        let raw = m.raw.lock().unwrap()[0].clone();
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("name=\"model\"\r\n\r\nlarge-v3-turbo"), "{text}");
        let at = raw.windows(4).position(|w| w == b"RIFF").expect("a WAV file in the upload");
        assert_eq!(&raw[at + 8..at + 16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(raw[at + 24..at + 28].try_into().unwrap()), 16_000, "the sample rate");
        assert_eq!(u32::from_le_bytes(raw[at + 40..at + 44].try_into().unwrap()), 10, "five samples are ten bytes");
        assert_eq!(i16::from_le_bytes(raw[at + 44 + 8..at + 44 + 10].try_into().unwrap()), -32768);
    }

    #[test]
    fn the_screen_steps_are_local_only_and_report_failures_as_text() {
        use crate::screen::{png_bytes, wav_bytes, HttpDescriber, HttpTranscriber};
        use xiar_screen::pipeline::{Clip, Describer};
        assert!(HttpDescriber::new(HttpEngineConfig::new("http://api.example.com", "m", 1), "p").is_err());
        assert!(HttpTranscriber::new(HttpEngineConfig::new("https://127.0.0.1:1", "m", 1)).is_err());
        let m = mock(vec![(500, "boom".into()), (200, json!({"nope": 1}).to_string())]);
        let d = HttpDescriber::new(cfg(&m.url), "p").unwrap();
        assert!(d.describe(&frame_2x2()).unwrap_err().contains("500"));
        assert!(d.describe(&frame_2x2()).unwrap_err().contains("no text"));
        assert!(d.describe(&xiar_screen::pipeline::Frame { ts_ms: 0, width: 3, height: 3, rgba: vec![0; 4], app: String::new(), title: String::new() }).is_err(), "a malformed frame is refused before any request");
        assert!(png_bytes(0, 0, &[]).is_err());
        assert_eq!(wav_bytes(8_000, &[]).len(), 44);
        let _ = Clip { sample_rate: 8_000, pcm16: vec![] };
    }
}
