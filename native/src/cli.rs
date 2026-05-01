use std::time::Instant;

use async_openai::Client;
use async_openai::config::OpenAIConfig;
use futures::StreamExt;
use portable::{ChatEvent, ChatRequest, Message, MessageRole, Theme, TokenUsage, estimate_tokens};
use thiserror::Error;
use tracing::{debug, error, info, instrument, trace};

use crate::AppState;
use crate::cli::display::{
    DisplayError, LiveMarkdown, build_user_prompt, get_duration, print_banner,
};
use crate::cli::prompt::{PromptError, read_multiline, select_model};
use crate::models::EnrichedModel;
use crate::openai::{ProviderError, get_chat_event, send_chat_request};

mod display;
mod prompt;

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
pub async fn run_cli(state: AppState, theme: &Theme) -> Result<(), CliError> {
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
            - *Ctrl+Enter* submits\n\
            - *Ctrl+C* cancels\n\
            \n\
            ---\n",
        );

        // let the user select a system prompt, or exit
        let Some(system_prompt) =
            // TODO: apply theme to reedline ?
            read_multiline("System prompt", Some(&state.default_system_prompt.clone())).await?
        else {
            return Ok(());
        };
        termimad::print_text("\n---\n");

        // Initialize history with system prompt
        let history = vec![Message::new(MessageRole::System, system_prompt)];
        debug!(message=?history[0], "system prompt");

        // Run the chat to completion
        run_chat(&state.openai_client, &selected_model, history, theme).await?;
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub(crate) async fn run_chat(
    client: &Client<OpenAIConfig>,
    selected_model: &EnrichedModel,
    mut history: Vec<portable::Message>,
    theme: &Theme,
) -> Result<(), CliError> {
    // smart token display (exact > approximate)
    let mut token_count = TokenUsage::default();
    loop {
        // update token usage with estimate if nothing better
        token_count.set_approximate(estimate_tokens(&history));

        // get user input
        let prompt = build_user_prompt(
            &selected_model.id,
            &token_count,
            selected_model.info.context_window,
            theme,
        );
        let input = match read_multiline(&prompt, None).await? {
            Some(input) => {
                // until we succeed
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

        // prepare chat request
        history.push(Message::new(MessageRole::User, input));
        let chat = ChatRequest::new(selected_model.id.to_string(), history.clone());
        let start = Instant::now();
        let reply = handle_chat(client, &chat, theme, |event| {
            // handle streaming events
            debug!(event = ?event, "chat event");
            match event {
                ChatEvent::TokenCount {
                    prompt,
                    generated,
                    // not displayed in CLI for now
                    cached: _cached,
                    reasoning: _reasoning,
                } => {
                    token_count.set_exact(prompt + generated);
                }
                ChatEvent::Error(msg) => {
                    error!(error = msg, "run_chat Streaming error");
                }
                _ => {}
            }
        })
        .await?;
        println!("{}", get_duration(start, theme));
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
    mut on_event: impl FnMut(&ChatEvent),
) -> Result<String, CliError> {
    // Can only fail here during initial request (setup)
    let mut stream = send_chat_request(&client, chat).await?;

    let mut full = String::new();
    let mut live = LiveMarkdown::new(theme);

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
