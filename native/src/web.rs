use std::convert::Infallible;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response, sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{StreamExt, stream};
use portable::{ChatEvent, ChatRequest, ConfigDto, Message, MessageRole, ModelDto};
use thiserror::Error;
use tower_http::services::ServeDir;
use tracing::{error, instrument};

use crate::AppState;
use crate::openai::{ProviderError, get_chat_event, send_chat_request};

#[derive(Error, Debug)]
enum WebError {
    #[error("API error {0}")]
    Api(#[from] ProviderError),
    #[error("Forbidden {0}")]
    Forbidden(String),
}

/// Allows for Axum to use our error type as responses
impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Api(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
        };
        (status, self.to_string()).into_response()
    }
}

// ── Web entrypoint ────────────────────────────────────────────────────────────

pub async fn run_web(state: AppState, port: &u16, dist_wasm: &str) -> Result<(), std::io::Error> {
    let app = router(state, dist_wasm);
    let listen_addr = format!("localhost:{port}");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    println!("Server listening on {listen_addr}");
    axum::serve(listener, app).await?;
    Ok(())
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

fn router(state: AppState, dist_wasm: &str) -> Router {
    Router::new()
        .route("/api/config", get(handle_config))
        .route("/api/models", get(handle_models))
        .route("/api/chat", post(handle_chat))
        .fallback_service(ServeDir::new(dist_wasm).append_index_html_on_directories(true))
        .with_state(state)
        .maybe_cors_permissive()
}

// ── GET /api/config ───────────────────────────────────────────────────────────

#[instrument(skip_all)]
async fn handle_config(State(s): State<AppState>) -> Json<ConfigDto> {
    Json(ConfigDto {
        prepend_system_prompt: s.prepend_system_prompt.as_ref().clone(),
    })
}

// ── GET /api/models ───────────────────────────────────────────────────────────

#[instrument(skip_all)]
async fn handle_models(State(s): State<AppState>) -> Json<Vec<ModelDto>> {
    Json(
        s.available_models
            .iter()
            .map(|(model_id, model_info)| ModelDto {
                id: model_id.clone(),
                context_window: model_info.context_window,
            })
            .collect(),
    )
}

// ── POST /api/chat ────────────────────────────────────────────────────────────

#[instrument(level = "trace", skip_all)]
async fn handle_chat(
    State(s): State<AppState>,
    Json(mut chat): Json<ChatRequest>,
) -> Result<sse::Sse<stream::BoxStream<'static, Result<sse::Event, Infallible>>>, WebError> {
    // CRITICAL/SECURITY
    // server-side check that the client is not trying to jail out
    if s.available_models.get(&chat.model).is_none() {
        Err(WebError::Forbidden(format!(
            "Configuration does not allow model '{}'",
            chat.model
        )))?
    }

    // Prepend the system prompt to the one provided by the client
    let prepend = s.prepend_system_prompt.trim();

    // TODO: remove this part once we change the input crate and actually
    //       give power to the user to not be limited BY HIS OWN config !
    if !prepend.is_empty() {
        match chat
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
                chat.messages.insert(0, default_sys_msg);
            }
        }
    }

    // Can only fail here during initial request (setup)
    let openai_stream = send_chat_request(s.openai_client.as_ref(), &chat).await?;

    // Every time a chunk is ready, the provided FnMut will be called
    // on it and and return a *Future* (axum needs a future) to poll,
    // which will then be run to completion to produce sse::Event
    let sse_stream = openai_stream.then(
        // the returned stream must be 'static because we Box::pin it,
        // so the closure cannot borrow from the enclosing scope, and
        // should own everything it references (hence this 'move')
        move |chunk| {
            // returns an async block the caller will need to await, and
            // async must be 'static to be polled, so 'move' to own
            async move {
                // as seen in function signature, this returns a Result
                // of sse::Event and Infallible, so no Err? possible
                // To report error during chat processing :
                // - server-side : use log with higher severity
                // - client-side : send ChatEvent::Error via SSE
                let event = get_chat_event(chunk);
                Ok(SseEventOut::from(event).into())
            }
        },
    );

    // Fixed memory is needed so that it can safely be referenced and
    // polled asynchronously. So move the stream onto the heap (Box)
    // and mark it as fixed in memory space
    // And we return it as a box trait object "without a lifetime" ('static)
    // so that it can live for as long as necessary in the async event loop
    Ok(sse::Sse::new(Box::pin(sse_stream)))
}

// ── SSE Event helper ──────────────────────────────────────────────────────────

struct SseEventOut(ChatEvent);

impl From<ChatEvent> for SseEventOut {
    fn from(e: ChatEvent) -> Self {
        SseEventOut(e)
    }
}

impl From<SseEventOut> for sse::Event {
    fn from(ev: SseEventOut) -> Self {
        let ev = ev.into_inner();
        let value = match &ev {
            ChatEvent::MessageToken(token) => serde_json::to_string(&token),
            ChatEvent::FinishReason { reason, refusal } => {
                #[derive(serde::Serialize)]
                struct Tmp<T> {
                    reason: T,
                    refusal: Option<T>,
                }
                serde_json::to_string(&Tmp {
                    reason,
                    refusal: refusal.as_ref(),
                })
            }
            ChatEvent::Error(err_msg) => serde_json::to_string(&err_msg),
            ChatEvent::TokenCount { prompt, generated } => {
                #[derive(serde::Serialize)]
                struct Tmp<T> {
                    prompt: T,
                    generated: T,
                }
                serde_json::to_string(&Tmp { prompt, generated })
            }
        };
        // this serialization should never fail (on our side), but serde might.
        match value {
            Ok(value) => sse::Event::default().event(ev.as_ref()).data(value),
            Err(e) => {
                error!(event=?ev, error=?e.to_string(), "Unexpected SSE event json serialization error");
                // build an infallible event, with consistent event name and hardcoded json encoding
                let msg = String::from(r#""ChatEvent serialization SSE error, see server log""#);
                let event = ChatEvent::Error(msg.clone());
                sse::Event::default().event(event.as_ref()).data(msg)
            }
        }
    }
}

impl SseEventOut {
    fn into_inner(self) -> ChatEvent {
        self.0
    }
}
