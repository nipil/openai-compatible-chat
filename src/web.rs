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
use axum::{
    Json, Router,
    extract::State,
    response::sse::{Event, Sse},
    routing::{get, post},
};
use futures::{StreamExt, stream::BoxStream};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use crate::{
    config::{self, Exclusion, Mapping},
    models,
};

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub client: Arc<Client<OpenAIConfig>>,
    pub mapping: Arc<Mapping>,
    pub exclusion: Arc<RwLock<Exclusion>>, // shared mutable exclusion list
    pub filters: Arc<Vec<Regex>>,
    pub system_prompt: String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/models", get(handle_models))
        .route("/api/chat", post(handle_chat))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ── GET /api/models ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ModelDto {
    id: String,
    family: String,
    model_type: Option<String>,
    max_tokens: Option<u32>,
}

async fn handle_models(State(s): State<AppState>) -> Json<Vec<ModelDto>> {
    let excl = s.exclusion.read().await;
    let ids = models::list_models(&s.client).await.unwrap_or_default();
    let enriched = models::filter_and_sort(ids, &s.mapping, &excl.excluded_models, &s.filters);
    Json(
        enriched
            .into_iter()
            .map(|m| ModelDto {
                id: m.id,
                family: m.family,
                model_type: m.model_type,
                max_tokens: m.max_tokens,
            })
            .collect(),
    )
}

// ── POST /api/chat ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<MessageDto>,
}

#[derive(Deserialize)]
pub struct MessageDto {
    pub role: String,
    pub content: String,
}

async fn handle_chat(
    State(s): State<AppState>,
    Json(mut req): Json<ChatRequest>,
) -> Sse<BoxStream<'static, Result<Event, Infallible>>> {
    if !s.system_prompt.is_empty()
        && req.messages.first().map(|m| m.role.as_str()) != Some("system")
    {
        req.messages.insert(
            0,
            MessageDto {
                role: "system".into(),
                content: s.system_prompt.clone(),
            },
        );
    }

    let stream: BoxStream<'static, Result<Event, Infallible>> =
        match build_chat_stream(s, req).await {
            Ok(s) => s,
            Err(e) => Box::pin(futures::stream::once(async move {
                Ok(Event::default().event("error").data(e.to_string()))
            })),
        };

    Sse::new(stream)
}

async fn build_chat_stream(
    s: AppState,
    req: ChatRequest,
) -> anyhow::Result<BoxStream<'static, Result<Event, Infallible>>> {
    let messages = req
        .messages
        .iter()
        .map(msg_to_api)
        .collect::<anyhow::Result<Vec<_>>>()?;

    let request = CreateChatCompletionRequestArgs::default()
        .model(&req.model)
        .messages(messages)
        .build()?;

    let openai_stream = s.client.chat().create_stream(request).await?;

    // Capture what we need for the exclusion side-effect inside the stream
    let model = req.model.clone();
    let exclusion = Arc::clone(&s.exclusion);

    // Use `.then()` (async map) so we can do async writes on unauthorized errors
    let sse = openai_stream.then(move |chunk| {
        let model = model.clone();
        let exclusion = Arc::clone(&exclusion);
        async move {
            match chunk {
                Ok(resp) => {
                    let token = resp
                        .choices
                        .first()
                        .and_then(|c| c.delta.content.clone())
                        .unwrap_or_default();
                    Ok::<Event, Infallible>(Event::default().data(token))
                }

                Err(ref e) if is_unauthorized_model(e) => {
                    // Persist the newly excluded model and update shared state
                    let mut excl = exclusion.write().await;
                    if !excl.excluded_models.contains(&model) {
                        excl.excluded_models.push(model.clone());
                        // save_exclusion is sync; acceptable for infrequent disk writes
                        if let Err(io) = config::save_exclusion(&excl) {
                            eprintln!("Failed to persist exclusion list: {io}");
                        }
                    }
                    Ok(Event::default()
                        .event("error")
                        .data(format!("Model '{model}' has been excluded: {e}")))
                }

                Err(e) => Ok(Event::default().event("error").data(e.to_string())),
            }
        }
    });

    Ok(Box::pin(sse))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn msg_to_api(m: &MessageDto) -> anyhow::Result<ChatCompletionRequestMessage> {
    Ok(match m.role.as_str() {
        "system" => ChatCompletionRequestSystemMessageArgs::default()
            .content(m.content.as_str())
            .build()?
            .into(),
        "assistant" => ChatCompletionRequestAssistantMessageArgs::default()
            .content(m.content.as_str())
            .build()?
            .into(),
        _ => ChatCompletionRequestUserMessageArgs::default()
            .content(m.content.as_str())
            .build()?
            .into(),
    })
}

/// Detects API errors that indicate this model is not accessible to this key.
/// Refine this to match whatever patterns your test_model() already handles.
fn is_unauthorized_model(e: &OpenAIError) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("model_not_found")
        || msg.contains("does not exist")
        || msg.contains("not have access")
        || msg.contains("permission")
}
