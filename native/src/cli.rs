use std::sync::{Arc, RwLock};
use std::time::Instant;

use async_openai::Client;
use async_openai::config::OpenAIConfig;
use futures::StreamExt;
use portable::{ChatEvent, ChatRequest, Message, MessageRole, Theme, estimate_tokens};
use thiserror::Error;
use tracing::{debug, error, info, instrument, trace};

use crate::AppState;
use crate::cli::display::{DisplayError, LiveMarkdown, get_duration, print_banner};
use crate::cli::prompt::{PromptError, PromptState, read_multiline, select_model};
use crate::cli::reedline::AppPrompt;
use crate::cli::themes::ConsoleColors;
use crate::openai::{ProviderError, get_chat_event, send_chat_request};

mod display;
mod prompt;
mod reedline;
mod themes;

pub const DEFAULT_CLI_REFRESH_INTERVAL_MS: u64 = 100;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("API error {0}")]
    Api(#[from] ProviderError),
    #[error("Display error {0}")]
    Display(#[from] DisplayError),
    #[error("Prompt error {0}")]
    Prompt(#[from] PromptError),
}

/// Run chat session until user quits or error
pub async fn run_cli(state: AppState, theme: &Theme, refresh_ms: u64) -> Result<(), CliError> {
    loop {
        // let the user select a model, or exit
        // TODO: allow using theme in dialoguer::FuzzySelect ?
        let Some(selected_model) = select_model(state.available_models.as_ref()).await? else {
            info!("User cancelation, exiting.");
            return Ok(());
        };

        // display the models properties
        print_banner(&selected_model, theme);

        // display the help for the input system
        termimad::print_text(
            "\n\
            ---\n\
            **Multiline editor**\n\n\
            - *Enter* adds a line\n\
            - *Ctrl+Enter* or *Ctrl+D* submits\n\
            - *Ctrl+C* cancels\n\
            \n\
            ---\n",
        );

        // build the app prompt for the application
        let prompt = AppPrompt {
            colors: Arc::new(ConsoleColors::new(theme)),
            refresh_ms: Arc::new(refresh_ms),
            state: Arc::new(RwLock::new(PromptState::new(selected_model))),
            theme: Arc::new(theme.clone()),
        };

        // let the user select a system prompt, or exit
        let Some(system_prompt) =
            read_multiline(prompt.clone(), Some(&state.default_system_prompt.clone())).await?
        else {
            return Ok(());
        };
        termimad::print_text("\n---\n");

        // guard is dropped immediately
        prompt
            .state
            .write()
            .expect("Prompt `state` RwLock should not be poisonned")
            // Now the system prompt is set, show that we are in user mode
            .current_role = MessageRole::User;

        // Initialize history with system prompt
        let history = vec![Message::new(MessageRole::System, system_prompt)];
        debug!(message=?history[0], "system prompt");

        // Run the chat to completion
        run_chat(&state.openai_client, prompt, history).await?;
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub(crate) async fn run_chat(
    client: &Client<OpenAIConfig>,
    prompt: AppPrompt,
    mut history: Vec<portable::Message>,
) -> Result<(), CliError> {
    // smart token display (exact > approximate)
    loop {
        // guard is dropped immediately
        prompt
            .state
            .write()
            .expect("Prompt `state` RwLock should not be poisonned")
            .token_usage
            // update token usage with estimate (if nothing better is inside)
            .set_approximate(estimate_tokens(&history));

        // get the user input until we succeed
        let input = match read_multiline(prompt.clone(), None).await? {
            Some(input) => {
                if input.trim().len() == 0 {
                    continue;
                }
                input
            }
            // or he quits
            // TODO: ask confirmation ?
            None => return Ok(()),
        };
        debug!(input = input, "user input");
        termimad::print_text("\n---\n");

        // guard is dropped immediately
        let model_id = prompt
            .state
            .read()
            .expect("Prompt `state` RwLock should not be poisonned")
            .selected_model
            .id
            .clone();

        // prepare chat request
        history.push(Message::new(MessageRole::User, input));
        let chat = ChatRequest::new(model_id, history.clone());
        let start = Instant::now();

        // local action on some streaming events
        let on_event = |event: &ChatEvent| {
            debug!(event = ?event, "chat event");

            match event {
                ChatEvent::TokenCount {
                    prompt: token_prompt,
                    generated,
                    // not displayed in CLI for now
                    cached: _cached,
                    reasoning: _reasoning,
                } => {
                    // guard is dropped immediately
                    prompt
                        .state
                        .write()
                        .expect("Prompt `state` RwLock should not be poisonned")
                        .token_usage
                        .set_exact(token_prompt + generated);
                }

                ChatEvent::Error(msg) => {
                    error!(error = msg, "run_chat Streaming error");
                }

                _ => {}
            }
        };

        // handle the streaing answers and most events
        let reply = handle_chat(
            client,
            &chat,
            &prompt.theme,
            *prompt.refresh_ms.as_ref(),
            on_event,
        )
        .await?;

        println!("{}", get_duration(start, &prompt.theme));
        termimad::print_text("---");

        debug!(reply = reply, "reply");
        history.push(Message::new(MessageRole::Assistant, reply));
    }
}

// ── Streaming response ────────────────────────────────────────────────────────

/// One request to the provider (only initial request, not streaming response)
#[instrument(level = "debug", skip_all)]
async fn handle_chat(
    client: &Client<OpenAIConfig>,
    chat: &ChatRequest,
    theme: &Theme,
    refresh_ms: u64,
    mut on_event: impl FnMut(&ChatEvent),
) -> Result<String, CliError> {
    // Can only fail here during initial request (setup)
    let mut stream = send_chat_request(&client, chat).await?;

    let mut full = String::new();
    let mut live = LiveMarkdown::new(theme, refresh_ms);

    while let Some(chunk) = stream.next().await {
        // forward model to enhance the cache token logging in openai module
        let event = get_chat_event(chunk, &chat.model);
        on_event(&event);
        if let ChatEvent::MessageToken(ref delta) = event {
            trace!(delta = ?delta, "delta");
            full.push_str(delta);
        }
        live.update(&full);
    }

    live.finish(&full);

    Ok(full)
}
