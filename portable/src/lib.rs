use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strum::{AsRefStr, Display, EnumString};

// ── OpenAI info ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    Chat,
    Multimodal,
    Reasoning,
    Instruct,
}

// ── Safer value management ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
// TODO: add strum?
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
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
    #[serde(default)]
    pub exclude_model_name_regex: Vec<String>, // TODO: make regex ?
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

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Exclusion {
    #[serde(default)]
    pub excluded_models: Vec<String>,
}

// ── DTOs ──────────────────────────────────────────────────────────────────────

// PartialEq required by a bound in `leptos::prelude::Memo::<T>::new`
#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct ModelDto {
    pub id: String,
    pub context_window: Option<u32>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ConfigDto {
    pub system_prompt: String,
    pub locked_model: Option<String>,
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
