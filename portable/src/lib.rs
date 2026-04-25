use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};
use thiserror::Error;

#[derive(Deserialize, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
}

// ── Safer value management ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Display, EnumString, AsRefStr)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    Assistant,
    System,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Display, EnumString, AsRefStr)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
}

// ── SSE ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SseError {
    #[error("unknown value: {0}")]
    Strum(#[from] strum::ParseError),
    #[error("json error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Display, EnumString, AsRefStr)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SseEventKind {
    MessageToken,
    TokenCount,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Display, EnumString, AsRefStr)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SseEvent {
    MessageToken(String),
    TokenCount { prompt: usize, generated: usize },
    Error(String),
}

// ── DTOs ──────────────────────────────────────────────────────────────────────

// PartialEq required by leptos Memo
// Clone required by leptos RwSignal.get()
#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct ModelDto {
    pub id: String,
    pub context_window: Option<u32>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ConfigDto {
    pub prepend_system_prompt: String,
}

/// We reuse the same structure for :
/// - Frontend <-> Backend
/// - Backend <-> OpenAI-compatible provider
/// Might need to split it if they diverge
#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

// ── Token estimate ────────────────────────────────────────────────────────────

/// Rough estimate: 1 token ≈ 4 UTF-8 chars, 3 tokens overhead per message,
/// plus 3 for the reply primer — mirrors the Python fallback heuristic.
pub fn estimate_tokens(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| 3 + m.content.chars().count() / 4)
        .sum::<usize>()
        + 3
}
