use std::sync::Arc;

use async_openai::Client;
use async_openai::config::OpenAIConfig;

use crate::models::EnrichedModels;

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
    pub candidate_models: Arc<EnrichedModels>,
}
