use std::fmt::Display;

use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};
use thiserror::Error;

pub const OPENAI_CACHE_TOKEN_THRESHOLD: u32 = 1024;

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

// ── Chat ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
}

impl ChatRequest {
    pub fn new(model: String, messages: Vec<Message>) -> Self {
        Self { model, messages }
    }
}

#[derive(Debug, Error)]
pub enum ChatEventError {
    #[error("unknown value: {0}")]
    Strum(#[from] strum::ParseError),

    #[error("json error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Display, EnumString, AsRefStr)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ChatEventKind {
    MessageToken,
    FinishReason,
    TokenCount,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Display, EnumString, AsRefStr)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ChatEvent {
    MessageToken(String),

    FinishReason {
        reason: String,
        refusal: Option<String>,
    },

    TokenCount {
        prompt: u32,
        generated: u32,

        // above portable::OPENAI_CACHE_TOKEN_THRESHOLD only
        // and only when supported by the model
        cached: Option<u32>,

        reasoning: Option<u32>,
    },

    Error(String),
}

// ── DTOs ──────────────────────────────────────────────────────────────────────

// PartialEq required by leptos Memo
// Clone required by leptos RwSignal.get()
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub struct ModelDto {
    // IMPORTANT: the derive use declaration order as priority
    pub id: String,
    pub context_window: Option<u32>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ConfigDto {
    pub default_system_prompt: String,
}

/// We reuse the same structure for :
/// - Frontend <-> Backend
/// - Backend <-> OpenAI-compatible provider
/// Might need to split it if they diverge
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

impl Message {
    pub fn new(role: MessageRole, content: String) -> Self {
        Self { role, content }
    }
}

// ── Token estimate ────────────────────────────────────────────────────────────

/// Rough estimate: 1 token ≈ 4 UTF-8 chars, 3 tokens overhead per message,
/// plus 3 for the reply primer — mirrors the Python fallback heuristic.
pub fn estimate_tokens(messages: &[Message]) -> u32 {
    messages
        .iter()
        .map(|m| 3u32 + m.content.chars().count() as u32 / 4u32)
        .sum::<u32>()
        + 3
}

#[derive(Debug, Clone)]
pub enum TokenUsage {
    Exact(u32),
    Approximate(u32),
}

impl PartialEq for TokenUsage {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Exact(a), Self::Exact(b)) => a == b,
            (Self::Approximate(a), Self::Approximate(b)) => a == b,
            _ => false,
        }
    }
}

impl Display for TokenUsage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Approximate(value) => write!(f, "~{}", value),
            Self::Exact(value) => write!(f, "{}", value),
        }
    }
}

impl Default for TokenUsage {
    fn default() -> Self {
        Self::Approximate(0)
    }
}

impl From<&TokenUsage> for u32 {
    fn from(usage: &TokenUsage) -> u32 {
        match usage {
            TokenUsage::Exact(v) => *v,
            TokenUsage::Approximate(v) => *v,
        }
    }
}

impl TokenUsage {
    pub fn set_exact(&mut self, total: u32) {
        // always override with API provided value
        *self = TokenUsage::Exact(total);
    }

    pub fn set_approximate(&mut self, total: u32) {
        // update only if we are already approximate
        if let TokenUsage::Approximate(_) = self {
            *self = TokenUsage::Approximate(total);
        }
    }
}
