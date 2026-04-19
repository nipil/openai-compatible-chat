use crate::config;
use crate::models::{self, ModelError};
use async_openai::{
    Client,
    config::OpenAIConfig,
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
use portable::{ConfigDto, Exclusion, Message, MessageRole, ModelDto, ModelInfoMap};
use regex::Regex;
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc};
use tokio::sync::RwLock;
use tower_http::services::ServeDir;

// TODO: make configurable using Clap
const DIST_FOLDER: &str = "wasm/dist";
const SSE_EVENT_ERROR: &str = "error";

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub client: Arc<Client<OpenAIConfig>>,
    pub infos: Arc<ModelInfoMap>,
    pub exclusion: Arc<RwLock<Exclusion>>, // shared mutable exclusion list
    pub filters: Arc<Vec<Regex>>,
    pub system_prompt: Arc<String>,
    pub locked_model: Arc<Option<String>>, // set via CLI --model, None means free choice
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(state: AppState) -> Router {
    let router = Router::new()
        .route("/api/config", get(handle_config))
        .route("/api/models", get(handle_models))
        .route("/api/chat", post(handle_chat))
        .fallback_service(ServeDir::new(DIST_FOLDER).append_index_html_on_directories(true))
        .with_state(state);
    #[cfg(feature = "cors-permissive")]
    {
        use tower_http::cors::CorsLayer;
        router.layer(CorsLayer::permissive())
    }
    #[cfg(not(feature = "cors-permissive"))]
    router
}

// ── GET /api/config ───────────────────────────────────────────────────────────

async fn handle_config(State(s): State<AppState>) -> Json<ConfigDto> {
    Json(ConfigDto {
        system_prompt: s.system_prompt.as_ref().clone(),
        locked_model: s.locked_model.as_ref().clone(),
    })
}

// ── GET /api/models ───────────────────────────────────────────────────────────

async fn handle_models(State(s): State<AppState>) -> Json<Vec<ModelDto>> {
    let exclusion = s.exclusion.read().await;
    // TODO: do not fetch every time, reuse from main
    let ids = models::list_models(&s.client).await.unwrap_or_default();
    // TODO: DRY in regards to main call too ?
    let mut enriched =
        models::filter_and_sort(ids, &s.infos, &exclusion.excluded_models, &s.filters);
    // If a model is locked, expose only that model in the list
    if let Some(lm) = s.locked_model.as_ref() {
        enriched.retain(|m| &m.id == lm);
    }
    Json(enriched.iter().map(|m| m.into()).collect())
}

// ── POST /api/chat ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
}

async fn handle_chat(
    State(s): State<AppState>,
    Json(mut req): Json<ChatRequest>,
) -> Sse<BoxStream<'static, Result<Event, Infallible>>> {
    // Server-side model lock overrides whatever the client sent
    if let Some(lm) = s.locked_model.as_ref() {
        req.model = lm.clone();
    }

    // Inject system prompt if not already provided by the client
    if !s.system_prompt.is_empty()
        && req.messages.first().map(|m| &m.role) != Some(&MessageRole::System)
    {
        req.messages.insert(
            0,
            Message {
                role: MessageRole::System,
                content: s.system_prompt.as_ref().clone(),
            },
        );
    }

    let stream: BoxStream<'static, Result<Event, Infallible>> =
        match build_chat_stream(s, req).await {
            Ok(s) => s,
            // TODO: display error or not ?
            Err(e) => Box::pin(futures::stream::once(async move {
                Ok(Event::default().event(SSE_EVENT_ERROR).data(e.to_string()))
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
        .filter(|m| !(m.role == MessageRole::System && m.content.trim().is_empty()))
        .map(msg_to_api)
        // There are two possible way to collect "list of results" :
        // - collect::<Vec<Result<_,_>>>() → keep every result
        //   → what we would do if we wanted to log each error (for example)
        // - collect::<Result<Vec<_>>>() → first error wins, rest is ignored
        //   → what we do here, as we do nothing like logging each err
        // TODO: RECHECK once msg_to_api is not anyhow ... or make thiserror or openai or with_context
        .collect::<anyhow::Result<Vec<_>>>()?;

    let request = CreateChatCompletionRequestArgs::default()
        .model(&req.model)
        .messages(messages)
        .build()?; // TODO: thiserror or openai or with_context

    let openai_stream = s.client.chat().create_stream(request).await?; // TODO: thiserror or openai

    // Capture what we need for the exclusion side-effect inside the stream
    let model = req.model.clone();
    let exclusion = Arc::clone(&s.exclusion);
    let client = Arc::clone(&s.client);

    // Use `.then()` (async map) so we can do async writes on unauthorized errors
    let sse = openai_stream.then(move |chunk| {
        let model = model.clone();
        let exclusion = Arc::clone(&exclusion);
        let client = Arc::clone(&client);
        async move {
            match chunk {
                Ok(resp) => {
                    let token = resp
                        .choices
                        .first()
                        .and_then(|c| c.delta.content.clone())
                        .unwrap_or_default();
                    #[cfg(feature = "print-tokens")]
                    {
                        use std::io::{self, Write};
                        print!("{token}");
                        io::stdout().flush().unwrap();
                    }
                    // encode the token in json so that newlines in the token
                    // does not break the SSE frame, and are preserved to frontend
                    // but the frontend should now decode the json data
                    let token = serde_json::to_string(&token).unwrap_or_default();
                    Ok::<Event, Infallible>(Event::default().data(token))
                }

                Err(e) => {
                    // Reuse test_model's exact detection logic to check if this
                    // model is unauthorized, rather than re-parsing the error string
                    if let Err(ModelError::NotAllowed) = models::test_model(&client, &model).await {
                        let mut exclusion = exclusion.write().await;
                        if !exclusion.excluded_models.contains(&model) {
                            exclusion.excluded_models.push(model.clone());
                            if let Err(io) = config::save_model_id_exclusion_list(&exclusion) {
                                eprintln!("Failed to persist exclusion list: {io}");
                            }
                        }
                        return Ok(Event::default()
                            .event(SSE_EVENT_ERROR)
                            .data(format!("Model '{model}' has been excluded: {e}")));
                    }
                    Ok(Event::default().event(SSE_EVENT_ERROR).data(e.to_string()))
                }
            }
        }
    });

    Ok(Box::pin(sse))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

// TODO: make DRY and deduplicate vs chat.rs
// TODO: thiserror OpenAiError ? or not because of defaults ?
fn msg_to_api(m: &Message) -> anyhow::Result<ChatCompletionRequestMessage> {
    Ok(match m.role {
        MessageRole::System => ChatCompletionRequestSystemMessageArgs::default()
            .content(m.content.as_str())
            .build()?
            .into(),
        MessageRole::Assistant => ChatCompletionRequestAssistantMessageArgs::default()
            .content(m.content.as_str())
            .build()?
            .into(),
        MessageRole::User => ChatCompletionRequestUserMessageArgs::default()
            .content(m.content.as_str())
            .build()?
            .into(),
    })
}
