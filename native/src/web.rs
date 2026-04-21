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
    http::StatusCode,
    response::IntoResponse,
    response::sse::{Event, Sse},
    routing::{get, post},
};
use futures::{StreamExt, stream::BoxStream};
use portable::{ConfigDto, EnrichedModel, Message, MessageRole, ModelDto};
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc};
use tower_http::services::ServeDir;

// TODO: make configurable using Clap
const DIST_FOLDER: &str = "wasm/dist";
const SSE_EVENT_ERROR: &str = "error";

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub client: Arc<Client<OpenAIConfig>>,
    pub prepend_system_prompt: Arc<String>,
    pub allowed_models: Arc<Vec<EnrichedModel>>,
}

// ── Router ────────────────────────────────────────────────────────────────────

trait RouterExt {
    fn maybe_cors_permissive(self) -> Self;
}

impl RouterExt for Router {
    fn maybe_cors_permissive(self) -> Self {
        #[cfg(feature = "cors-permissive")]
        {
            return self.layer(tower_http::cors::CorsLayer::permissive());
        }
        self
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/config", get(handle_config))
        .route("/api/models", get(handle_models))
        .route("/api/chat", post(handle_chat))
        .fallback_service(ServeDir::new(DIST_FOLDER).append_index_html_on_directories(true))
        .with_state(state)
        .maybe_cors_permissive()
}

// ── GET /api/config ───────────────────────────────────────────────────────────

async fn handle_config(State(s): State<AppState>) -> Json<ConfigDto> {
    Json(ConfigDto {
        prepend_system_prompt: s.prepend_system_prompt.as_ref().clone(),
    })
}

// ── GET /api/models ───────────────────────────────────────────────────────────

async fn handle_models(State(s): State<AppState>) -> Json<Vec<ModelDto>> {
    Json(s.allowed_models.iter().map(|m| m.into()).collect())
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
) -> Result<Sse<BoxStream<'static, Result<Event, Infallible>>>, impl IntoResponse> {
    // CRITICAL: server-side check that the client is not trying to screw us
    // check that the requested in the allowed model list of the valid types
    if !s
        .allowed_models
        .iter()
        // TODO: switch Vec<EnrichedModel> to HashMap<String, EnrichedModel>
        .any(|m| m.id == req.model)
    {
        let res = (
            StatusCode::FORBIDDEN,
            format!("Configuration does not allow model '{}'", req.model),
        );
        return Err(res);
    }

    // TODO: implement pathological cases here if needed (huge payload)
    // TODO: implement message-based busines logic here (logging)

    // Prepend the system prompt to the one provided by the client
    let prepend = s.prepend_system_prompt.trim();
    // TODO: remove this part once we change the input crate and actually
    //       give power to the user to not be limited BY HIS OWN config !
    if !prepend.is_empty() {
        match req
            .messages
            .iter_mut()
            .find(|m| m.role == MessageRole::System)
        {
            Some(msg) => {
                // paragraph separation improves intent detection for models
                msg.content = format!("{}\n\n{}", prepend, msg.content.trim());
            }
            None => {
                let default_sys_msg = Message {
                    role: MessageRole::System,
                    content: prepend.to_string(),
                };
                req.messages.insert(0, default_sys_msg);
            }
        }
    }

    let stream: BoxStream<'static, Result<Event, Infallible>> =
        match build_chat_stream(s, req).await {
            Ok(s) => s,
            Err(e) => {
                // INFORMATION
                // Here we can not get any Error from the chunk
                // processing future, as it is managed from its
                // SSE stream then() closure. And the stream
                // is not even streamed here, as we return it back
                // to our caller, which is an Axum route handler.
                // And anyway, that closure is marked as Infallible
                // anyway, as it only returns Ok and rust knows it !

                // IMPORTANT
                // During the creation of the SSE stream itself,
                // we could get errors which we can handle:
                // - msg_to_api() could fail as these could :
                //   - ChatCompletionRequestSystemMessageArgs.build()
                //   - ChatCompletionRequestAssistantMessageArgs.build()
                //   - ChatCompletionRequestUserMessageArgs.build()
                // - CreateChatCompletionRequestArgs.build()
                // But as we build them from default() and only add
                // valid stuff, it most likely could not anyway. But
                // there is no reason the async_openai crate could not
                // fail on its own, so we have to handle this anyway.
                Box::pin(futures::stream::once(async move {
                    // TODO: logging ?
                    // On server-side error **DURING SSE CREATION**,
                    // send an SSE "error event" to notify the client
                    Ok(Event::default().event(SSE_EVENT_ERROR).data(e.to_string()))
                }))
            }
        };

    Ok(Sse::new(stream))
}

async fn build_chat_stream(
    s: AppState,
    req: ChatRequest,
) -> anyhow::Result<BoxStream<'static, Result<Event, Infallible>>> {
    let messages = req
        .messages
        .iter()
        // remove empty system messages to avoid confusing the model with empty instructions
        .filter(|m| !(m.role == MessageRole::System && m.content.trim().is_empty()))
        .map(msg_to_api)
        // There are two possible way to collect "list of results" :
        // - collect::<Vec<Result<_,_>>>() → keep every result
        //   → what we would do if we wanted to log each error (for example)
        // - collect::<Result<Vec<_>>>() → first error wins, rest is ignored
        //   → what we do here, as we do nothing like logging each err
        // TODO: RECHECK once msg_to_api is not anyhow ... or make thiserror or openai or with_context
        .collect::<Result<Vec<_>, OpenAIError>>()?;

    let request = CreateChatCompletionRequestArgs::default()
        .model(&req.model)
        .messages(messages)
        .build()?; // TODO: thiserror or openai or with_context

    let openai_stream = s.client.chat().create_stream(request).await?; // TODO: thiserror or openai

    // Use `.then()` (async map) so we can do async writes on unauthorized errors
    let sse = openai_stream.then(move |chunk| {
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
                        use std::io::{Write, stdout};
                        // TODO: switch to write to handle failures
                        print!("{token}");
                        stdout().flush().unwrap();
                    }
                    // encode the token in json so that newlines in the token
                    // does not break the SSE frame, and are preserved to frontend
                    // but the frontend should now decode the json data
                    let token = serde_json::to_string(&token).unwrap_or_default();
                    Ok::<Event, Infallible>(Event::default().data(token))
                }

                Err(e) => {
                    // TODO: logging ?
                    // On server-side error **DURING CHUNKS PROCESSING**,
                    // send an SSE "error event" to notify the client
                    Ok(Event::default().event(SSE_EVENT_ERROR).data(e.to_string()))
                }
            }
        }
    });

    Ok(Box::pin(sse))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

// TODO: thiserror OpenAiError ? or not because of defaults ?
pub fn msg_to_api(m: &Message) -> Result<ChatCompletionRequestMessage, OpenAIError> {
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
