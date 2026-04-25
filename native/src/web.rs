use std::convert::Infallible;

use anyhow::Result;
use async_openai::error::OpenAIError;
use async_openai::types::chat::{
    ChatCompletionResponseStream, CreateChatCompletionStreamResponse, FinishReason,
};
// TODO: anyhow should not be used in lib crate,only thiserror
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{StreamExt, stream};
use portable::{ChatRequest, ConfigDto, Message, MessageRole, ModelDto, SseError, SseEvent};
use tower_http::services::ServeDir;
use tracing::{debug, error, info, instrument, trace, warn};

use crate::AppState;
use crate::openai::{ProviderError, send_for_stream};

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
    Json(mut req): Json<ChatRequest>,
) -> Result<sse::Sse<stream::BoxStream<'static, Result<sse::Event, Infallible>>>, impl IntoResponse>
{
    // CRITICAL: server-side check that the client is not trying to screw us
    // check that the requested in the allowed model list of the valid types
    if !s
        .candidate_models
        .keys()
        // TODO: switch Vec<EnrichedModel> to HashMap<String, EnrichedModel>
        .any(|model_id| model_id == &req.model)
    {
        let res = (
            StatusCode::FORBIDDEN,
            format!("Configuration does not allow model '{}'", req.model),
        );
        return Err(res);
    }

    // FIXME: reuse this logic for the cli version

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

    let stream: stream::BoxStream<'static, Result<sse::Event, Infallible>> =
        // only awaits the setup (the initial API call to get the stream handle)
        // no chunk are processed yet and wil be done by the caller
        // which, as the stream is Send + 'static, can be an async fn (axum)

        match build_chat_stream(s, &req).await {
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

pub struct SseEventOut(SseEvent);

impl From<SseEvent> for SseEventOut {
    fn from(e: SseEvent) -> Self {
        SseEventOut(e)
    }
}

impl TryFrom<SseEventOut> for sse::Event {
    type Error = SseError;

    fn try_from(ev: SseEventOut) -> Result<Self, Self::Error> {
        let ev = ev.into_inner();
        let kind = ev.as_ref();
        let value = serde_json::to_string(&ev)?;
        Ok(sse::Event::default().event(kind).data(value))
    }
}

impl SseEventOut {
    pub fn into_inner(self) -> SseEvent {
        self.0
    }
}

/// One request to the provider (only initial request, not streaming response)
#[instrument(level = "trace", skip_all)]
async fn build_chat_stream(
    s: AppState,
    chat: &ChatRequest,
) -> Result<stream::BoxStream<'static, Result<sse::Event, Infallible>>, ProviderError> {
    // TODO: refactor same as cli ? into openai
    // not mut here because the client is holding the history and provides it

    // This response stream will produce "Item" and every item is of type
    // Result<CreateChatCompletionStreamResponse, _>
    let stream: ChatCompletionResponseStream =
        send_for_stream(s.openai_client.as_ref(), chat).await?;

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
        move |chunk: Result<CreateChatCompletionStreamResponse, OpenAIError>| {
            // same reasoning, async should own its reference to be 'static
            async move {
                match chunk {
                    Ok(resp) => {
                        // response can have multiple choice, according to request config
                        match resp.choices.len() {
                            0 => {
                                // when usage was requested in the request,
                                // there is a chunk with empty choice
                                if let Some(usage) = resp.usage {
                                    debug!(
                                        prompt = usage.prompt_tokens,
                                        completion = usage.completion_tokens,
                                        // TODO: use this value to display in message
                                        // FIXME: find a way to provide it to the caller?
                                        total = usage.total_tokens,
                                        "Token usage"
                                    );
                                }
                            }
                            1 => {
                                trace!(response = ?resp, "response");
                            }
                            _ => {
                                warn!(id = resp.id,
                                    count = resp.choices.len(),
                                    response = ?resp,
                                    "using first choice of many");
                            }
                        };

                        // in all cases, only process the first one
                        let Some(choice) = resp.choices.first() else {
                            // FIXME: better feedback and check in frontend how it behaves
                            return Ok(sse::Event::default().data("NO\nCHOICE\r\nFOR\ryou!\n"));
                        };

                        // TODO: auto print from serde_serialize + trim quotes ?
                        match choice.finish_reason {
                            None => {}
                            Some(reason) => match reason {
                                FinishReason::Stop => {
                                    debug!(id = resp.id, reason = "stop", "finish")
                                }
                                FinishReason::Length => {
                                    warn!(id = resp.id, reason = "length", "finish")
                                }
                                FinishReason::ToolCalls => {
                                    info!(id = resp.id, reason = "tool_calls", "finish")
                                }
                                FinishReason::ContentFilter => {
                                    error!(id = resp.id, reason = "content_filter", "finish")
                                }
                                FinishReason::FunctionCall => {
                                    info!(id = resp.id, reason = "function_call", "finish")
                                }
                            },
                        }

                        let token = choice
                            .delta
                            .content
                            .as_ref()
                            .and_then(|c| Some(c.clone()))
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
                        // let token = serde_json::to_string(&token).unwrap_or_default();
                        Ok::<sse::Event, Infallible>(sse::Event::default().data(token))
                    }

                    Err(e) => {
                        // TODO: logging ?
                        // On server-side error **DURING CHUNKS PROCESSING**,
                        // send an SSE "error event" to notify the client
                        Ok(sse::Event::default()
                            .event(SSE_EVENT_ERROR)
                            .data(e.to_string()))
                    }
                }
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
