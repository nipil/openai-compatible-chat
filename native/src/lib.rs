use crate::models::EnrichedModel;
use async_openai::{Client, config::OpenAIConfig};
use std::sync::Arc;

pub mod config;
pub mod models;
pub mod openai;

#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "web")]
pub mod web;

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub openai_client: Arc<Client<OpenAIConfig>>,
    pub prepend_system_prompt: Arc<String>,
    pub allowed_models: Arc<Vec<EnrichedModel>>,
}
