use thiserror::Error;
use tracing::info;

use crate::AppState;
use crate::cli::chat::{ChatError, print_banner, run_chat};
use crate::cli::display::{DisplayError, select_model};

mod chat;
mod display;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("Chat error {0}")]
    Chat(#[from] ChatError),
    #[error("Display error {0}")]
    Display(#[from] DisplayError),
}

/// Run chat session until user quits or error
pub async fn run_cli(state: AppState) -> Result<(), CliError> {
    loop {
        let Some(selected_model) = select_model(state.candidate_models.as_ref())? else {
            info!("User cancelation, exiting.");
            return Ok(());
        };
        print_banner(&selected_model);
        run_chat(
            &state.openai_client,
            &selected_model,
            &state.prepend_system_prompt,
        )
        .await?;
    }
}
