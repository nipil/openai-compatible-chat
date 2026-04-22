use crate::cli::chat::{ChatOutcome, run_chat};
use crate::cli::display::{log_critical, log_info, log_warning, select_model};
use anyhow::{Result, anyhow}; // TODO: anyhow should not be used in lib crate,only thiserror
use async_openai::{Client, config::OpenAIConfig};
use portable::EnrichedModel;

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
            log_info("User cancel.");
            return Ok(());
        };
        match run_chat(
            &client,
            &allowed_models[selected_index],
            &prepend_system_prompt,
        )
        .await?
        {
            ChatOutcome::ChatEnded => {
                log_info("Chat ended.");
                continue;
            }
            ChatOutcome::ContextLimitReached => {
                log_warning("Context limit reached — starting a new conversation.");
                continue;
            }
            ChatOutcome::ExitRequested => {
                log_info("Requested to quit");
                return Ok(());
            }
            ChatOutcome::ModelForbidden => {
                if allowed_models.len() > 1 {
                    log_warning("Model is forbidden, choose another one.");
                    continue;
                } else {
                    log_critical("The only available model is forbidden, exiting.");
                    return Err(anyhow!("No more model available to use."));
                }
            }
        }
    }
}
