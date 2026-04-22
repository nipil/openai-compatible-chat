use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt::Display};
use strum::{AsRefStr, Display, EnumString};

// ── OpenAI info ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    Audio,
    Chat,
    Completion,
    Embedding,
    Image,
    Instruct,
    Moderation,
    Multimodal,
    Realtime,
    Reasoning,
    Search,
    Transcription,
    Video,
}

#[derive(Debug, Clone)]
pub struct EnrichedModel {
    pub id: String,
    pub info: ModelInfo,
}

impl Display for EnrichedModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.id, self.info.model_type)
    }
}

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

// ── Configurations ────────────────────────────────────────────────────────────

pub type ModelInfoMap = HashMap<String, ModelInfo>;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    #[serde(with = "serde_regex")]
    pub exclude_model_name_regex: Vec<Regex>,
    #[serde(default)]
    pub prepend_system_prompt: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelInfo {
    pub description: String,
    pub family: String,
    #[serde(rename = "type")]
    pub model_type: ModelType,
    pub context_window: Option<u32>,
    pub release: Option<String>,
}

// ── DTOs ──────────────────────────────────────────────────────────────────────

// PartialEq required by leptos Memo
// Clone required by leptos RwSignal.get()
#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct ModelDto {
    pub id: String,
    pub context_window: Option<u32>,
}

impl From<&EnrichedModel> for ModelDto {
    fn from(other: &EnrichedModel) -> Self {
        Self {
            id: other.id.clone(),
            context_window: other.info.context_window,
        }
    }
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
