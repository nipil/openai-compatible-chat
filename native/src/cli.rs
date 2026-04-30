use portable::{Message, MessageRole};
use thiserror::Error;
use tracing::{debug, info};

use crate::AppState;
use crate::cli::chat::{ChatError, run_chat};
use crate::cli::display::{DisplayError, print_banner, select_model};
use crate::cli::prompt::{PromptError, read_multiline};

mod chat;
mod display;
mod prompt;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("Chat error {0}")]
    Chat(#[from] ChatError),
    #[error("Display error {0}")]
    Display(#[from] DisplayError),
    #[error("Prompt error {0}")]
    Prompt(#[from] PromptError),
}

/// Run chat session until user quits or error
pub async fn run_cli(state: AppState) -> Result<(), CliError> {
    loop {
        // let the user select a model, or exit
        let Some(selected_model) = select_model(state.available_models.as_ref())? else {
            info!("User cancelation, exiting.");
            return Ok(());
        };

        // display the models properties
        print_banner(&selected_model);
        termimad::print_text("");

        // display the help for the input system
        termimad::print_text(
            "\n\
            ---\n\
            **Multiline editor**\n\n\
            - *Enter* adds a line\n\
            - *Ctrl+Enter* submits\n\
            - *Ctrl+C* cancels\n\
            \n\
            ---\n",
        );

        // let the user select a system prompt, or exit
        let Some(system_prompt) =
            read_multiline("System prompt", Some(&state.default_system_prompt.clone()))?
        else {
            continue;
        };
        termimad::print_text("\n---\n");

        // Initialize history with system prompt
        let history = vec![Message::new(MessageRole::System, system_prompt)];
        debug!(message=?history[0], "system prompt");

        // Run the chat to completion
        run_chat(&state.openai_client, &selected_model, history).await?;
    }
}
