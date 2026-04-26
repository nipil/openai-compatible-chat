use std::convert::Infallible;

use anyhow::Result;
use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::error::OpenAIError;
use async_openai::types::chat::{ChatCompletionResponseStream, CreateChatCompletionStreamResponse};
// TODO: anyhow should not be used in lib crate,only thiserror
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{StreamExt, stream};
use portable::{ChatEvent, ChatRequest, ConfigDto, Message, MessageRole, ModelDto};
use tower_http::services::ServeDir;
use tracing::{error, instrument};

use crate::AppState;
use crate::openai::{ProviderError, get_chat_event, send_chat_request};

const SSE_EVENT_ERROR: &str = "error";

// ── Web entrypoint ────────────────────────────────────────────────────────────

pub async fn run_web(state: AppState, port: &u16, dist_wasm: &str) -> Result<()> {
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

async fn handle_config(State(s): State<AppState>) -> Json<ConfigDto> {
    Json(ConfigDto {
        prepend_system_prompt: s.prepend_system_prompt.as_ref().clone(),
    })
}

// ── GET /api/models ───────────────────────────────────────────────────────────

async fn handle_models(State(s): State<AppState>) -> Json<Vec<ModelDto>> {
    Json(
        s.candidate_models
            .iter()
            .map(|(model_id, model_info)| ModelDto {
                id: model_id.clone(),
                context_window: model_info.context_window,
            })
            .collect(),
    )
}

// ── POST /api/chat ────────────────────────────────────────────────────────────

// TODO: clarify the base-errors that are used
async fn handle_chat(
    State(s): State<AppState>,
    Json(mut chat): Json<ChatRequest>,
) -> Result<sse::Sse<stream::BoxStream<'static, Result<sse::Event, Infallible>>>, impl IntoResponse>
{
    // CRITICAL: server-side check that the client is not trying to screw us
    // check that the requested in the allowed model list of the valid types
    if !s
        .candidate_models
        .keys()
        // TODO: switch Vec<EnrichedModel> to HashMap<String, EnrichedModel>
        .any(|model_id| model_id == &chat.model)
    {
        let res = (
            StatusCode::FORBIDDEN,
            format!("Configuration does not allow model '{}'", chat.model),
        );
        return Err(res);
    }

    // FIXME: reuse this logic for the cli version

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

    let stream: stream::BoxStream<'static, Result<sse::Event, Infallible>> =
        // only awaits the setup (the initial API call to get the stream handle)
        // no chunk are processed yet and wil be done by the caller
        // which, as the stream is Send + 'static, can be an async fn (axum)

        match process_chat_stream(s.openai_client.as_ref(), &chat).await {
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
                    Ok(sse::Event::default()
                        .event(SSE_EVENT_ERROR)
                        .data(e.to_string()))
                }))
            }
        };

    // we return an actual stream, which axum will iterate over, and
    // each time, will pull one chunk, run the closure asynchronously
    Ok(sse::Sse::new(stream))
}

// #[derive(Debug, Error)]
// pub enum ChatError {
//     #[error("provider error: {0}")]
//     ProviderError(#[from] openai::ProviderError),
// }

// TODO: move to openai once similar to chat::send_and_stream

// ── SSE Event helper ──────────────────────────────────────────────────────────

pub struct SseEventOut(ChatEvent);

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
    pub fn into_inner(self) -> ChatEvent {
        self.0
    }
}

/// One request to the provider (only initial request, not streaming response)
#[instrument(level = "trace", skip_all)]
async fn process_chat_stream(
    client: &Client<OpenAIConfig>,
    chat: &ChatRequest,
) -> Result<stream::BoxStream<'static, Result<sse::Event, Infallible>>, ProviderError> {
    // TODO: refactor same as cli ? into openai
    // not mut here because the client is holding the history and provides it

    // This response stream will produce "Item" and every item is of type
    // Result<CreateChatCompletionStreamResponse, _>
    let stream: ChatCompletionResponseStream = send_chat_request(client, chat).await?;

    // Use `.then()` (async map) so we can do async writes on unauthorized errors

    // to get data out of the future, we can
    // - get it as a final value once it is resolved, after await
    // - clone an Arc<Mutex<T>> and move it in the closure
    // - clone a tokio mpsc channel annd moge it in the closure

    let sse: stream::Then<ChatCompletionResponseStream, _, _> = stream.then(
        // Every time an "Item" is ready, the provided FnMut will be called
        // with Item, and return a *Future*, which will then be run to completion
        // to produce the next value on this stream.

        // the returned stream must be 'static because we Box::pin it,
        // so the closure cannot borrow from the enclosing scope, and
        // should own everything it references.
        // Except if it does not reference anything from the enclosing scope
        |chunk: Result<CreateChatCompletionStreamResponse, OpenAIError>| {
            // same reasoning, async should own its reference to be 'static
            async {
                let event = get_chat_event(chunk);
                Ok(SseEventOut::from(event).into())
            }
        },
    );

    // Moves the Thenable onto the heap (Box) and mark it as fixed in memory
    // space, so that its memory address cannot change : it is needed so that
    // it can safely be referenced and polled
    Ok(Box::pin(sse))
}

// {} block — evaluates immediately, produces the closure as its value
// move |chunk| — the closure, matches F: FnMut(...) -> Fut
// async move {} — an async block (not a closure), matches Fut: Future
