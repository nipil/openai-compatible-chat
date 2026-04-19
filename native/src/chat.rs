use crate::display::LiveMarkdown;
use anyhow::Result;
use async_openai::{
    Client,
    config::OpenAIConfig,
    error::OpenAIError,
    types::chat::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs,
    },
};
use chrono::Local;
use futures::StreamExt;
use owo_colors::OwoColorize;
use portable::{Config, EnrichedModel, Message, MessageRole, estimate_tokens};
use std::{
    fmt,
    io::{Write, stdin, stdout},
    time::Instant,
};
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator};

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

pub async fn run(
    client: &Client<OpenAIConfig>,
    model: &str,
    models_meta: &[EnrichedModel],
    config: &Config,
) -> Result<ChatOutcome> {
    println!("\n{}\n", "─── Conversation ───".white().bold());

    let system = build_system_prompt(config).await;
    let mut history = vec![Message {
        role: MessageRole::System,
        content: system,
    }];

    let max_tokens = models_meta
        .iter()
        .find(|m| m.id == model)
        .and_then(|m| m.info.context_window);

    loop {
        let input = read_user_input_trimmed(model, &history, max_tokens).await;
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

        match send_and_stream(client, model, &history).await {
            Ok(reply) => {
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
                    crate::display::log_warning(&format!("Model '{model}' not allowed"));
                    return Ok(ChatOutcome::ModelForbidden);
                }
                crate::display::log_error(&e.to_string());
            }
        }
    }
}

// ── Prompt / stdin ────────────────────────────────────────────────────────────

async fn build_system_prompt(config: &Config) -> String {
    let user = tokio::task::spawn_blocking(|| {
        print!("System prompt: ");
        stdout().flush().ok();
        let mut buf = String::new();
        stdin().read_line(&mut buf).ok();
        buf.trim().to_string()
    })
    .await
    .unwrap_or_default();

    let pre = config.prepend_system_prompt.trim();
    match (pre.is_empty(), user.is_empty()) {
        (true, _) => user,
        (false, true) => pre.to_string(),
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

async fn send_and_stream(
    client: &Client<OpenAIConfig>,
    model: &str,
    history: &[Message],
) -> Result<String, OpenAIError> {
    let messages = to_api_messages(history)?;
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

// TODO: make DRY and deduplicate vs web.rs
// TODO: thiserror OpenAiError ? or not because of defaults
fn to_api_messages(history: &[Message]) -> Result<Vec<ChatCompletionRequestMessage>, OpenAIError> {
    history
        .iter()
        .map(|m| {
            Ok(match m.role {
                MessageRole::System => ChatCompletionRequestSystemMessageArgs::default()
                    .content(m.content.as_str())
                    .build()?
                    .into(),
                MessageRole::User => ChatCompletionRequestUserMessageArgs::default()
                    .content(m.content.as_str())
                    .build()?
                    .into(),
                MessageRole::Assistant => ChatCompletionRequestAssistantMessageArgs::default()
                    .content(m.content.as_str())
                    .build()?
                    .into(),
            })
        })
        .collect()
}
