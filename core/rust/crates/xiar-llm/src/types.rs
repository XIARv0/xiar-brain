// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool invocation requested by the model. `arguments` is the raw JSON text exactly as the model produced it, so a
/// malformed payload can be reported back to the model instead of being silently repaired.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: Option<String>,
    pub name: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(text: &str) -> Self {
        Self { role: Role::System, content: Some(text.into()), name: None, tool_calls: vec![], tool_call_id: None }
    }
    pub fn user(text: &str) -> Self {
        Self { role: Role::User, content: Some(text.into()), name: None, tool_calls: vec![], tool_call_id: None }
    }
    pub fn assistant(text: Option<&str>, tool_calls: Vec<ToolCall>) -> Self {
        Self { role: Role::Assistant, content: text.map(str::to_string), name: None, tool_calls, tool_call_id: None }
    }
    pub fn tool_result(name: &str, call_id: &str, content: &str) -> Self {
        Self { role: Role::Tool, content: Some(content.into()), name: Some(name.into()), tool_calls: vec![], tool_call_id: Some(call_id.into()) }
    }
}

/// A callable tool as shown to the model (JSON-Schema parameters).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// force the model to call this tool (used for structured extraction such as topic extraction)
    pub force_tool: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, o: Usage) {
        self.prompt_tokens += o.prompt_tokens;
        self.completion_tokens += o.completion_tokens;
        self.total_tokens += o.total_tokens;
    }
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub message: ChatMessage,
    pub usage: Usage,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum LlmError {
    /// the prompt does not fit the model's context window: the agent reacts by summarising and retrying
    #[error("context window exceeded: {0}")]
    ContextOverflow(String),
    /// worth retrying (rate limit, timeout)
    #[error("transient engine error: {0}")]
    Transient(String),
    #[error("engine error: {0}")]
    Fatal(String),
    #[error("no engine configured for profile {0:?}")]
    NoEngine(String),
}

pub trait LlmEngine: Send + Sync {
    fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError>;
    /// context window in tokens
    fn context_window(&self) -> u32;
    fn model(&self) -> &str;
}

pub trait EmbedEngine: Send + Sync {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, LlmError>;
    fn dim(&self) -> u32;
    fn model(&self) -> &str;
    fn plugin(&self) -> &str;
}

/// Rough token estimate (~4 characters per token) used when an engine does not report usage.
pub fn estimate_tokens(text: &str) -> u32 {
    (text.chars().count() as u32).div_ceil(4)
}

pub fn estimate_request_tokens(req: &ChatRequest) -> u32 {
    let msgs: u32 = req
        .messages
        .iter()
        .map(|m| estimate_tokens(m.content.as_deref().unwrap_or("")) + m.tool_calls.iter().map(|t| estimate_tokens(&t.arguments) + estimate_tokens(&t.name)).sum::<u32>() + 4)
        .sum();
    let tools: u32 = req.tools.iter().map(|t| estimate_tokens(&t.description) + estimate_tokens(&t.parameters.to_string())).sum();
    msgs + tools
}
