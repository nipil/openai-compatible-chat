use anyhow::{Result, anyhow}; // TODO: anyhow should not be used in lib crate,only thiserror
use tracing::{error, info, warn};

use crate::AppState;
use crate::cli::chat::{ChatOutcome, run_chat};
use crate::cli::display::select_model;

pub mod chat;
pub mod display;

/// Run chat session until user quits or error
pub async fn run_cli(state: AppState) -> Result<()> {
    loop {
        let Some(selected_model) = select_model(state.candidate_models.as_ref())? else {
            info!("User cancelation, exiting.");
            return Ok(());
        };
        match run_chat(
            &state.openai_client,
            &selected_model,
            &state.prepend_system_prompt,
        )
        .await?
        {
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
                if state.candidate_models.len() > 1 {
                    warn!(model = selected_model.id, "Model not allowed");
                    continue;
                } else {
                    error!("The only available model is forbidden, exiting.");
                    return Err(anyhow!("No more model available to use."));
                }
            }
        }
    }
}
