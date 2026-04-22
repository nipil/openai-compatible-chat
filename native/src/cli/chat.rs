use crate::{cli::display::LiveMarkdown, models::EnrichedModel, openai::messages_to_api};
use anyhow::Result;
use async_openai::{
    Client, config::OpenAIConfig, error::OpenAIError, types::chat::CreateChatCompletionRequestArgs,
};
use chrono::Local;
use futures::StreamExt;
use owo_colors::OwoColorize;
use portable::{Message, MessageRole, estimate_tokens};
use std::{
    fmt,
    io::{Write, stdin, stdout},
    time::Instant,
};
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator};
use tracing::{error, warn};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, EnumString, EnumIter, AsRefStr)]
#[strum(serialize_all = "lowercase")]
pub enum ChatCommand {
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
) -> Result<ChatOutcome> {
    println!(
        "\n{} {} {}\n",
        "─── Conversation using".white().bold(),
        selected_model.id.cyan().bold(),
        "───".white().bold(),
    );
    let system = get_system_prompt_from_user(prepend_system_prompt).await;
    let mut history = vec![Message {
        role: MessageRole::System,
        content: system,
    }];

    loop {
        let input = read_user_input_trimmed(
            &selected_model.id,
            &history,
            selected_model.info.context_window,
        )
        .await;

        if input.is_empty() {
            continue;
        }

        if let Some(cmd) = ChatCommand::detect_from(&input) {
            match cmd {
                ChatCommand::New => return Ok(ChatOutcome::ChatEnded),
                ChatCommand::Quit => return Ok(ChatOutcome::ExitRequested),
                ChatCommand::Help => {
                    ChatCommand::iter().for_each(|x| println!("{x}"));
                    continue;
                }
            }
        }

        history.push(Message {
            role: MessageRole::User,
            content: input,
        });

        match send_and_stream(client, &selected_model.id, &history).await {
            Ok(reply) => {
                // upon successful completion only,
                // add the whole reply to the history,
                // to be sent for later messages
                history.push(Message {
                    role: MessageRole::Assistant,
                    content: reply,
                });
            }
            Err(e) => {
                history.pop(); // drop the unsatisfied user turn
                let msg = e.to_string().to_lowercase();
                // TODO: check error by type ?
                if msg.contains("context_length") {
                    return Ok(ChatOutcome::ContextLimitReached);
                }
                // TODO: deduplicate from main.rs:156
                if msg.contains("not allowed") || msg.contains("permission") {
                    warn!(model = selected_model.id, "Model  not allowed");
                    return Ok(ChatOutcome::ModelForbidden);
                }
                error!(exc = e.to_string(), "Error during streaming");
            }
        }
    }
}

// ── Prompt / stdin ────────────────────────────────────────────────────────────

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

async fn read_user_input_trimmed(
    model: &str,
    history: &[Message],
    max_tokens: Option<u32>,
) -> String {
    let tok = estimate_tokens(history);
    let now = Local::now().format("%H:%M:%S").to_string();
    let model = model.to_string();

    tokio::task::spawn_blocking(move || {
        let prompt = build_prompt_str(&now, &model, tok, max_tokens);
        print!("{prompt}");
        stdout().flush().ok();
        let mut buf = String::new();
        stdin().read_line(&mut buf).ok();
        buf.trim().to_string()
    })
    .await
    .unwrap_or_default()
}

fn build_prompt_str(time: &str, model: &str, tokens: usize, max: Option<u32>) -> String {
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
// TODO: move to openai module
async fn send_and_stream(
    client: &Client<OpenAIConfig>,
    model: &str,
    messages: &[Message],
) -> Result<String, OpenAIError> {
    // TODO: refactor same as cli ? into openai
    let messages = messages_to_api(messages)?;
    let req = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages(messages)
        .stream(true)
        .build()?;

    let mut stream = client.chat().create_stream(req).await?;

    let mut full = String::new();
    let mut live = LiveMarkdown::new();
    let start = Instant::now();
    println!();

    while let Some(event) = stream.next().await {
        let chunk = event?;
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
