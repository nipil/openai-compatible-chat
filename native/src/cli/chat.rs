use std::time::Instant;

use async_openai::Client;
use async_openai::config::OpenAIConfig;
use futures::StreamExt;
use portable::{ChatEvent, ChatRequest, Message, MessageRole, TokenUsage, estimate_tokens};
use thiserror::Error;
use tracing::{debug, error, instrument, trace};

use crate::cli::display::{LiveMarkdown, build_user_prompt, get_duration};
use crate::cli::prompt::{PromptError, read_multiline};
use crate::models::EnrichedModel;
use crate::openai::{ProviderError, get_chat_event, send_chat_request};

#[derive(Error, Debug)]
pub enum ChatError {
    #[error("API error {0}")]
    Api(#[from] ProviderError),
    #[error("Input error {0}")]
    Prompt(#[from] PromptError),
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run_chat(
    client: &Client<OpenAIConfig>,
    selected_model: &EnrichedModel,
    mut history: Vec<portable::Message>,
) -> Result<(), ChatError> {
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
        );
        let input = match read_multiline(&prompt, None)? {
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
        let reply = handle_chat(client, &chat, |event| {
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
        println!("{}", get_duration(start));
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
    mut on_event: impl FnMut(&ChatEvent),
) -> Result<String, ChatError> {
    // Can only fail here during initial request (setup)
    let mut stream = send_chat_request(&client, chat).await?;

    let mut full = String::new();
    let mut live = LiveMarkdown::new();

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
