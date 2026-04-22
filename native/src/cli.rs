use crate::cli::{
    chat::{ChatOutcome, run_chat},
    display::select_model,
};
use crate::models::EnrichedModel;
use anyhow::{Result, anyhow}; // TODO: anyhow should not be used in lib crate,only thiserror
use async_openai::{Client, config::OpenAIConfig};
use tracing::{error, info, warn};

pub mod chat;
pub mod display;

pub async fn run_cli(
    client: Client<OpenAIConfig>,
    allowed_models: Vec<EnrichedModel>,
    prepend_system_prompt: String,
) -> Result<()> {
    loop {
        // ── Run chat session ────────────────────────────────────────────────
        let Some(selected_index) = select_model(&allowed_models)? else {
            info!("User cancelation, exiting.");
            return Ok(());
        };
        let selected_model = &allowed_models[selected_index];
        match run_chat(&client, selected_model, &prepend_system_prompt).await? {
            ChatOutcome::ChatEnded => {
                info!("Chat ended.");
                continue;
            }
            ChatOutcome::ContextLimitReached => {
                warn!("Context limit reached — starting a new conversation.");
                continue;
            }
            ChatOutcome::ExitRequested => {
                info!("Exit requested.");
                return Ok(());
            }
            ChatOutcome::ModelForbidden => {
                if allowed_models.len() > 1 {
                    warn!(model = selected_model.id, "Model is forbidden");
                    continue;
                } else {
                    error!("The only available model is forbidden, exiting.");
                    return Err(anyhow!("No more model available to use."));
                }
            }
        }
    }
}
