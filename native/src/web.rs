use std::convert::Infallible;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response, sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{StreamExt, stream};
use portable::{ChatEvent, ChatRequest, ConfigDto, ModelDto};
use rust_embed::RustEmbed;
use thiserror::Error;
use tracing::{debug, error, instrument};

use crate::AppState;
use crate::openai::{ProviderError, get_chat_event, send_chat_request};

#[cfg(not(feature = "embed"))]
const DEFAULT_WASM_DIST: &str = "wasm/dist";

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
            return self.layer(tower_http::cors::CorsLayer::permissive());
        }
        self
    }

    fn fallback_static_assets(self) -> Self {
        #[cfg(feature = "embed")]
        {
            self.fallback_service(tower::service_fn(|req: Request| async move {
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

// ── Embedding files into binary ───────────────────────────────────────────────

/// The path resolution works as follows:
/// - In debug and when debug-embed feature is not enabled, the folder path is
///   resolved relative to where the binary is run from.
/// - In release or when debug-embed feature is enabled, the folder path is
///   resolved relative to where Cargo.toml is.
#[derive(RustEmbed)]
#[folder = "../wasm/dist"]
struct Assets;

fn serve_asset(path: &str) -> Result<Response, Infallible> {
    let mut path = path.trim_start_matches('/');
    if path.is_empty() {
        path = "index.html";
    }
    let response = match Assets::get(path) {
        Some(content) => {
            debug!(file = path, "Asset found");
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(content.data))
        }
        None => {
            debug!(file = path, "Asset not found");
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Not Found"))
        }
    };

    Ok(response.expect("Hardcoded response should not fail"))
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
