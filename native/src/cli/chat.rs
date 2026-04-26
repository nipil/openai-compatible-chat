use std::fmt;
use std::io::{Write, stdin, stdout};
use std::time::Instant;

use async_openai::Client;
use async_openai::config::OpenAIConfig;
use chrono::Local;
use futures::StreamExt;
use owo_colors::OwoColorize;
use portable::{ChatRequest, Message, MessageRole, estimate_tokens};
use strum::{AsRefStr, EnumIter, EnumString};
use thiserror::Error;
use tracing::instrument;

use crate::cli::display::LiveMarkdown;
use crate::models::EnrichedModel;
use crate::openai::{ProviderError, send_chat_request};

#[derive(Error, Debug)]
enum ChatError {
    #[error("API error {0}")]
    Api(#[from] ProviderError),
}

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, EnumString, EnumIter, AsRefStr)]
#[strum(serialize_all = "lowercase")]
enum ChatCommand {
    New,
    Quit,
    Help,
}

impl ChatCommand {
    fn detect_from(text: &str) -> Option<Self> {
        let mut text = text.trim_start().strip_prefix("/")?.trim_end();
        if let Some(space) = text.find(" ") {
            text = &text[..space];
        }
        Self::try_from(text).ok()
    }
}

impl fmt::Display for ChatCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}", self.as_ref())
    }
}

pub enum ChatOutcome {
    ChatEnded,
    ExitRequested,
    ModelForbidden,
    ContextLimitReached,
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run_chat<'a>(
    client: &Client<OpenAIConfig>,
    selected_model: &EnrichedModel<'a>,
    prepend_system_prompt: &str,
) -> Result<ChatOutcome, ProviderError> {
    // use system prompt to initialize history
    let system = get_system_prompt_from_user(prepend_system_prompt).await;
    let mut history = vec![Message {
        role: MessageRole::System,
        content: system,
    }];

    loop {
        // TODO: use the token from the usage event if available
        let token_count = estimate_tokens(&history);
        let input = get_user_input(selected_model, token_count).await;
        history.push(Message::new(MessageRole::User, input));
        let chat = ChatRequest::new(selected_model.id.to_string(), history.clone());
        let reply = interact_once(client, &chat, |tok| {
            print!("received token: {tok}");
            println!(" hist len {}", history.len());
        });
    }
}

async fn interact_once<'a>(
    client: &Client<OpenAIConfig>,
    chat: &ChatRequest,
    on_token: impl FnMut(&str),
) -> Result<String, ChatError> {
    // Can only fail here during initial request (setup)
    let openai_stream = send_chat_request(client, &chat).await?;
    send_and_stream(client, &chat).await
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

async fn get_system_prompt_from_user(prepend_system_prompt: &str) -> String {
    // TODO: switch to a readline crate whch will allow editing the default prompt before submitting
    let user = tokio::task::spawn_blocking(|| {
        print!("System prompt: ");
        // TODO: switch from print! to write! and bubble up all stout Err
        stdout().flush().ok();
        let mut buf = String::new();
        stdin().read_line(&mut buf).ok();
        buf.trim().to_string()
    })
    .await
    .unwrap_or_default();

    // TODO: once we use default instead of prepend, fix code below
    let pre = prepend_system_prompt.trim();
    match (pre.is_empty(), user.is_empty()) {
        (true, _) => user,
        (false, true) => pre.to_string(),
        // paragraph separation improves intent detection for models
        (false, false) => format!("{pre}\n\n{user}"),
    }
}

async fn get_user_input<'a>(selected_model: &EnrichedModel<'a>, token_count: u32) -> String {
    loop {
        let input = prompt_user_for_input(
            &selected_model.id,
            token_count,
            selected_model.info.context_window,
        )
        .await;
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        return input.into();
    }
}

async fn prompt_user_for_input(model: &str, tok_history: u32, max_tokens: Option<u32>) -> String {
    let now = Local::now().format("%H:%M:%S").to_string();
    let model = model.to_string();

    tokio::task::spawn_blocking(move || {
        let prompt = build_prompt_str(&now, &model, tok_history, max_tokens);
        print!("{prompt}");
        stdout().flush().ok();
        let mut buf = String::new();
        stdin().read_line(&mut buf).ok();
        buf.trim().to_string()
    })
    .await
    // TODO: thiserror / JoinError
    .unwrap_or_default()
}

fn build_prompt_str(time: &str, model: &str, tokens: u32, max: Option<u32>) -> String {
    let tok_coloured = match max {
        None => tokens.to_string().white().to_string(),
        Some(m) => {
            let r = tokens as f64 / m as f64;
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

    format!(
        "{}{}{}> ",
        format!("[{time}]").white(),
        format!("[{model}]").bright_black(),
        format!("[~{tok_coloured}]"),
    )
}

// ── Streaming response ────────────────────────────────────────────────────────

// TODO: refactor and provide a closure for the updates ?
// TODO: move to openai once similar to web::build_chat_stream

/// One request to the provider (only initial request, not streaming response)
#[instrument(level = "trace", skip_all)]
async fn send_and_stream(
    client: &Client<OpenAIConfig>,
    chat: &ChatRequest,
) -> Result<String, ProviderError> {
    // FIXME: this one awaits now, instead of a then with a closure as for the web
    // mut because in the CLI, we hold the history, not the browser
    let mut stream = send_chat_request(&client, chat).await?;

    let mut full = String::new();
    let mut live = LiveMarkdown::new();
    let start = Instant::now();
    println!();

    while let Some(event) = stream.next().await {
        let chunk = event.map_err(|e| ProviderError::StreamingError { source: e })?;
        for choice in &chunk.choices {
            if let Some(ref delta) = choice.delta.content {
                full.push_str(delta);
            }
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
