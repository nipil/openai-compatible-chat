use crate::{
    AppState,
    cli::{
        chat::{ChatOutcome, run_chat},
        display::select_model,
    },
};
use anyhow::{Result, anyhow}; // TODO: anyhow should not be used in lib crate,only thiserror
use tracing::{error, info, warn};

pub mod chat;
pub mod display;

pub async fn run_cli(state: AppState) -> Result<()> {
    loop {
        // ── Run chat session ────────────────────────────────────────────────
        let Some(selected_index) = select_model(&state.allowed_models)? else {
            info!("User cancelation, exiting.");
            return Ok(());
        };
        let selected_model = &state.allowed_models[selected_index];
        match run_chat(
            &state.openai_client,
            selected_model,
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
                if state.allowed_models.len() > 1 {
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
