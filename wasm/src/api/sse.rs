use std::str::FromStr;

use eventsource_stream::{Event, Eventsource};
use futures::StreamExt;
use js_sys::Uint8Array;
use portable::{
    ChatEvent, ChatEventError, ChatEventKind, ChatRequest, OPENAI_CACHE_TOKEN_THRESHOLD,
};
use strum::{AsRefStr, Display, EnumString};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_streams::ReadableStream;
use web_sys::{AbortSignal, Window};

use crate::{FutureStreamError, RequestError};

// module-private : used only in stream_chat
#[derive(Debug, Display, EnumString, AsRefStr)]
#[strum(serialize_all = "lowercase")]
enum HttpMethod {
    Post,
}

// module-private : SSE → ChatEvent conversion layer
struct SseEventIn(ChatEvent);

impl From<ChatEvent> for SseEventIn {
    fn from(e: ChatEvent) -> Self {
        SseEventIn(e)
    }
}

impl TryFrom<Event> for SseEventIn {
    type Error = ChatEventError;

    fn try_from(ev: Event) -> Result<Self, Self::Error> {
        let kind = ChatEventKind::from_str(&ev.event).map_err(ChatEventError::Strum)?;

        let event = match kind {
            ChatEventKind::MessageToken => {
                ChatEvent::MessageToken(serde_json::from_str::<String>(&ev.data)?)
            }

            ChatEventKind::FinishReason => {
                #[derive(serde::Deserialize)]
                struct Tmp<T> {
                    reason: T,
                    refusal: Option<T>,
                }

                let Tmp { reason, refusal } = serde_json::from_str(&ev.data)?;

                ChatEvent::FinishReason { reason, refusal }
            }

            ChatEventKind::TokenCount => {
                #[derive(serde::Deserialize)]
                struct Tmp<T> {
                    prompt: T,
                    generated: T,
                    cached: Option<T>,
                    reasoning: Option<T>,
                }

                let Tmp {
                    prompt,
                    generated,
                    cached,
                    reasoning,
                } = serde_json::from_str(&ev.data)?;

                ChatEvent::TokenCount {
                    prompt,
                    generated,
                    cached,
                    reasoning,
                }
            }

            ChatEventKind::Error => ChatEvent::Error(serde_json::from_str::<String>(&ev.data)?),
        };

        Ok(event.into())
    }
}

impl SseEventIn {
    fn into_inner(self) -> ChatEvent {
        self.0
    }
}

pub(crate) async fn stream_chat(
    window: Window,
    chat: ChatRequest,
    signal: AbortSignal,
    on_token: impl Fn(&str),
    on_total: impl Fn(u32),
) -> Result<(), RequestError> {
    let hdrs =
        web_sys::Headers::new().map_err(|e| RequestError::CreateHeaders { source: e.into() })?;
    hdrs.set("Content-Type", "application/json")
        .map_err(|e| RequestError::SetHeader { source: e.into() })?;

    let opts = web_sys::RequestInit::new();
    opts.set_method(HttpMethod::Post.as_ref());
    opts.set_headers(hdrs.as_ref());
    opts.set_body(&wasm_bindgen::JsValue::from_str(
        &serde_json::json!(chat).to_string(),
    ));
    opts.set_signal(Some(&signal));

    let req = web_sys::Request::new_with_str_and_init("/api/chat", &opts)
        .map_err(|e| RequestError::CreateRequest { source: e.into() })?;

    let resp: web_sys::Response = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|e| RequestError::FetchRequest { source: e.into() })?
        .dyn_into()
        .map_err(|e| RequestError::ConvertResponse { source: e.into() })?;

    if !resp.ok() {
        return Err(RequestError::HttpError {
            status: resp.status(),
            message: resp.status_text(),
        });
    }

    let websys_reader = resp.body().ok_or(RequestError::NoBody)?;

    let mut event_stream = ReadableStream::from_raw(websys_reader)
        .into_stream()
        .map(|chunk| {
            chunk
                .map(|v| Uint8Array::new(&v).to_vec())
                .map_err(|e| FutureStreamError::Chunk(e.into()))
        })
        .eventsource();

    while let Some(result) = event_stream.next().await {
        match result {
            Err(e) => return Err(RequestError::EventStream(e)),

            Ok(es_event) => {
                let sse_event = SseEventIn::try_from(es_event)?.into_inner();
                web_sys::console::debug_1(&format!("SSE event: {:?}", sse_event).into());

                match sse_event {
                    ChatEvent::TokenCount {
                        prompt,
                        generated,
                        cached,
                        reasoning,
                    } => {
                        on_total(prompt + generated);

                        web_sys::console::log_1(
                            &format!(
                                "Token count : prompt {prompt} answer={generated} \
                                reasoning={reasoning:?} cached(prompt>{})={cached:?}",
                                OPENAI_CACHE_TOKEN_THRESHOLD,
                            )
                            .into(),
                        );
                    }

                    ChatEvent::MessageToken(token) => on_token(&token),

                    ChatEvent::FinishReason { reason, refusal } => match refusal {
                        None => {
                            web_sys::console::info_1(&format!("Finish reason: {reason}").into())
                        }

                        Some(refusal) => web_sys::console::warn_1(
                            &format!("Finish reason: {reason} with {refusal}").into(),
                        ),
                    },

                    ChatEvent::Error(err_msg) => {
                        web_sys::console::error_1(&format!("SSE error: {err_msg}").into());
                    }
                }
            }
        }
    }

    Ok(())
}
