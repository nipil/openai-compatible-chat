use std::io::{Write, stdin, stdout};
use std::time::Instant;

use async_openai::Client;
use async_openai::config::OpenAIConfig;
use chrono::Local;
use futures::StreamExt;
use owo_colors::OwoColorize;
use portable::{ChatEvent, ChatRequest, Message, MessageRole, TokenUsage, estimate_tokens};
use thiserror::Error;
use tracing::{debug, error, instrument, trace};

use crate::cli::display::LiveMarkdown;
use crate::models::EnrichedModel;
use crate::openai::{ProviderError, get_chat_event, send_chat_request};

#[derive(Error, Debug)]
pub enum ChatError {
    #[error("API error {0}")]
    Api(#[from] ProviderError),
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run_chat<'a>(
    client: &Client<OpenAIConfig>,
    selected_model: &EnrichedModel<'a>,
    prepend_system_prompt: &str,
) -> Result<(), ChatError> {
    // use system prompt to initialize history
    let system = get_system_prompt(prepend_system_prompt).await;
    let mut history = vec![Message {
        role: MessageRole::System,
        content: system,
    }];
    debug!(message=?history[0], "system prompt");

    let mut token_count = TokenUsage::default();
    loop {
        token_count.set_approximate(estimate_tokens(&history));
        // get user input and prepare the request
        let input = get_user_prompt(selected_model, &token_count).await;
        debug!(input = input, "system prompt");
        println!("\n");
        history.push(Message::new(MessageRole::User, input));
        let chat = ChatRequest::new(selected_model.id.to_string(), history.clone());

        let reply = handle_chat(client, &chat, |event| {
            debug!(event = ?event, "chat event");
            match event {
                ChatEvent::TokenCount { prompt, generated } => {
                    token_count.set_exact(prompt + generated);
                }
                ChatEvent::Error(msg) => {
                    error!(error = msg, "run_chat Streaming error");
                }
                _ => {}
            }
        })
        .await?;
        debug!(reply = reply, "reply");
        history.push(Message::new(MessageRole::Assistant, reply));
    }
}

// ── Prompt / stdin ────────────────────────────────────────────────────────────

pub(crate) fn print_banner(selected_model: &EnrichedModel) {
    println!(
        "\n{} {} {}\n",
        "─── Conversation using".white().bold(),
        selected_model.id.cyan().bold(),
        "───".white().bold(),
    );
    let desc = selected_model.info.description.trim();
    if desc.len() > 0 {
        println!("description: {}", desc.white().italic());
    }
    let family = selected_model.info.family.trim();
    if desc.len() > 0 {
        println!("family: {}", family.white().italic());
    }
    if let Some(ref release) = selected_model.info.release {
        let release = release.trim();
        if release.len() > 0 {
            println!("release: {}", release.cyan().bold());
        }
    }
    println!();
}

async fn get_system_prompt(prepend_system_prompt: &str) -> String {
    // TODO: switch to a readline crate whch will allow editing the default prompt before submitting
    let user = prompt_line("System prompt: ").await;

    // TODO: once we use default instead of prepend, fix code below
    let pre = prepend_system_prompt.trim();
    match (pre.is_empty(), user.is_empty()) {
        (true, _) => user,
        (false, true) => pre.to_string(),
        // paragraph separation improves intent detection for models
        (false, false) => format!("{pre}\n\n{user}"),
    }
}

async fn get_user_prompt<'a>(
    selected_model: &EnrichedModel<'a>,
    token_count: &TokenUsage,
) -> String {
    loop {
        let input = prompt_user(
            &selected_model.id,
            token_count,
            selected_model.info.context_window,
        )
        .await;
        debug!(input=?input, "get_user_input");
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        return input.into();
    }
}

async fn prompt_line(prompt: &str) -> String {
    let prompt = prompt.to_string();
    tokio::task::spawn_blocking(move || {
        print!("{prompt}");
        stdout().flush().ok();
        let mut buf = String::new();
        stdin().read_line(&mut buf).ok();
        debug!(buffer=?buf, "prompt_line");
        buf.trim().to_string()
    })
    .await
    .unwrap_or_default()
}

async fn prompt_user(model: &str, tokens: &TokenUsage, max: Option<u32>) -> String {
    let time = Local::now().format("%H:%M:%S").to_string();
    let tok_coloured = match max {
        None => tokens.to_string().white().to_string(),
        Some(m) => {
            let r = u32::from(tokens) as f64 / m as f64;
            if r < 0.50 {
                tokens.to_string().bright_black().to_string()
            } else if r < 0.75 {
                tokens.to_string().white().to_string()
            } else if r < 0.90 {
                tokens.to_string().yellow().to_string()
            } else {
                tokens.to_string().red().to_string()
            }
        }
    };
    let prompt = format!(
        "{}{}{}> ",
        format!("[{time}]").white(),
        format!("[{model}]").bright_black(),
        format!("[{tok_coloured}]"),
    );
    prompt_line(&prompt).await
}

// ── Streaming response ────────────────────────────────────────────────────────

// TODO: refactor and provide a closure for the updates ?

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
    let start = Instant::now();

    while let Some(chunk) = stream.next().await {
        let event = get_chat_event(chunk);
        on_event(&event);
        if let ChatEvent::MessageToken(ref delta) = event {
            trace!(delta = delta, "delta");
            full.push_str(delta);
        }
        live.update(&full);
    }

    live.finish(&full);

    println!(
        "{}",
        format!("[{:.2}s]", start.elapsed().as_secs_f64()).bright_black()
    );
    println!();

    Ok(full)
}
