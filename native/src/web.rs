use std::convert::Infallible;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response, sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{StreamExt, stream};
use portable::{ChatEvent, ChatRequest, ConfigDto, ModelDto};
use thiserror::Error;
use tracing::{error, instrument};

use crate::AppState;
use crate::openai::{ProviderError, get_chat_event, send_chat_request};

#[cfg(not(feature = "embed"))]
const DEFAULT_WASM_DIST: &str = "wasm/dist";

#[cfg(feature = "embed")]
pub(crate) mod embed;

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
        // Axum uses .0 as http code and .1 as body
        (status, self.to_string()).into_response()
    }
}

// ── Web entrypoint ────────────────────────────────────────────────────────────

pub async fn run_web(state: AppState, bind_addr: &str, port: &u16) -> Result<(), std::io::Error> {
    let app = router(state);

    let listen_addr = format!("{bind_addr}:{port}");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    println!("Server listening on {listen_addr}");

    axum::serve(listener, app).await?;
    Ok(())
}

// ── Router ────────────────────────────────────────────────────────────────────

trait RouterExt {
    fn fallback_static_assets(self) -> Self;
    fn maybe_cors_permissive(self) -> Self;
}

impl RouterExt for Router {
    fn maybe_cors_permissive(self) -> Self {
        #[cfg(feature = "cors-permissive")]
        {
            use tower_http::cors::CorsLayer;
            return self.layer(CorsLayer::permissive());
        }
        #[cfg(not(feature = "cors-permissive"))]
        self
    }

    fn fallback_static_assets(self) -> Self {
        #[cfg(feature = "embed")]
        {
            use axum::extract::Request;
            use embed::serve_asset;
            use tower::service_fn;

            self.fallback_service(service_fn(|req: Request| async move {
                serve_asset(req.uri().path())
            }))
        }
        #[cfg(not(feature = "embed"))]
        {
            use tower_http::services::ServeDir;
            self.fallback_service(
                ServeDir::new(DEFAULT_WASM_DIST).append_index_html_on_directories(true),
            )
        }
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/config", get(handle_config))
        .route("/api/models", get(handle_models))
        .route("/api/chat", post(handle_chat))
        .with_state(state)
        .fallback_static_assets()
        .maybe_cors_permissive()
}

// ── GET /api/config ───────────────────────────────────────────────────────────

#[instrument(skip_all)]
async fn handle_config(State(s): State<AppState>) -> Json<ConfigDto> {
    Json(ConfigDto {
        default_system_prompt: s.default_system_prompt.as_ref().clone(),
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
    Json(chat): Json<ChatRequest>,
) -> Result<sse::Sse<stream::BoxStream<'static, Result<sse::Event, Infallible>>>, WebError> {
    // CRITICAL/SECURITY
    // server-side check that the client is not trying to jail out
    if s.available_models.get(&chat.model).is_none() {
        Err(WebError::Forbidden(format!(
            "Configuration does not allow model '{}'",
            chat.model
        )))?
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
            // forward it to enhance the cache token logging in openai module
            let model_id = chat.model.clone();
            // returns an async block the caller will need to await, and
            // async must be 'static to be polled, so 'move' to own
            async move {
                // as seen in function signature, this returns a Result
                // of sse::Event and Infallible, so no Err? possible
                // To report error during chat processing :
                // - server-side : use log with higher severity
                // - client-side : send ChatEvent::Error via SSE
                let event = get_chat_event(chunk, &model_id);
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
        // reused later
        let ev = ev.into_inner();

        // transform
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

            ChatEvent::TokenCount {
                prompt,
                generated,
                cached,
                reasoning,
            } => {
                #[derive(serde::Serialize)]
                struct Tmp<T> {
                    prompt: T,
                    generated: T,
                    cached: Option<T>,
                    reasoning: Option<T>,
                }

                serde_json::to_string(&Tmp {
                    prompt,
                    generated,
                    cached: cached.as_ref(),
                    reasoning: reasoning.as_ref(),
                })
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
