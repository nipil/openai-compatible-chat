use std::str::FromStr;

use eventsource_stream::{Event, Eventsource};
use futures::StreamExt;
use gloo_net::http::Request;
use js_sys::Uint8Array;
use leptos::mount::mount_to_body;
use leptos::prelude::*;
use leptos::task::spawn_local;
use portable::{
    ChatEvent, ChatEventError, ChatEventKind, ChatRequest, ConfigDto, Message, MessageRole,
    ModelDto, Theme, TokenUsage, estimate_tokens,
};
use send_wrapper::SendWrapper;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_streams::ReadableStream;
use web_sys::{AbortController, AbortSignal, KeyboardEvent, window};

// TODO: add tracing-wasm

const COOKIE_MODEL: &str = "model";
const COOKIE_THEME_DEFAULT: Theme = Theme::Dark;
const COOKIE_THEME: &str = "theme";
const STORAGE_KEY_OPENAI: &str = "openai";

// ── SSE Event helper ──────────────────────────────────────────────────────────

struct SseEventIn(ChatEvent);

impl From<ChatEvent> for SseEventIn {
    fn from(e: ChatEvent) -> Self {
        SseEventIn(e)
    }
}

impl TryFrom<Event> for SseEventIn {
    type Error = ChatEventError;

    fn try_from(ev: Event) -> Result<Self, Self::Error> {
        let kind = ChatEventKind::from_str(&ev.event).map_err(|e| ChatEventError::Strum(e))?;
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
                }
                let Tmp { prompt, generated } = serde_json::from_str(&ev.data)?;
                ChatEvent::TokenCount { prompt, generated }
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

// ── Token counter ─────────────────────────────────────────────────────────────

// Token counter: returns an inline style string for the dynamic gradient only.
// Static layout/padding lives in .token-counter in style.css.
fn token_color_style(count: &TokenUsage, max: Option<u32>) -> String {
    let (bg, color) = match max {
        None => ("rgba(128,128,128,0.4)", "var(--text-banner)"),
        Some(m) => {
            let pct = u32::from(count) as f64 / m as f64;
            if pct < 0.50 {
                ("transparent", "var(--text-banner)")
            } else if pct < 0.75 {
                ("#ffd700", "#333")
            } else if pct < 0.90 {
                ("#ff8c00", "white")
            } else {
                ("#cc0000", "white")
            }
        }
    };
    format!("background:{bg}; color:{color};")
}

// ── Markdown → HTML ───────────────────────────────────────────────────────────

fn to_html(md: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(md, opts));
    out
}

// ── Cookie helpers ────────────────────────────────────────────────────────────

fn get_cookie(name: &str) -> Option<String> {
    let cookies = web_sys::window()?
        .document()?
        .dyn_into::<web_sys::HtmlDocument>()
        .ok()?
        .cookie()
        .ok()?;
    cookies.split(';').find_map(|pair| {
        pair.trim()
            .strip_prefix(&format!("{name}="))
            .map(str::to_string)
    })
}

fn set_cookie(name: &str, value: &str) {
    let Some(windows) = web_sys::window() else {
        return;
    };
    let Some(document) = windows.document() else {
        return;
    };
    // max-age=31536000 → survives browser restarts for one year
    let Some(html_doc) = document.dyn_into::<web_sys::HtmlDocument>().ok() else {
        return;
    };
    let _ = html_doc.set_cookie(&format!(
        "{name}={value}; max-age=31536000; SameSite=Strict; path=/"
    ));
}

// ── Theme helpers ─────────────────────────────────────────────────────────────

fn apply_theme(theme: &Theme) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(element) = document.document_element() else {
        return;
    };
    let _ = element.set_attribute("data-theme", theme.as_ref());
}

fn get_cookie_theme_or_default() -> Theme {
    match get_cookie(COOKIE_THEME).as_deref().map(Theme::from_str) {
        Some(Ok(theme)) => theme,
        Some(Err(_)) => COOKIE_THEME_DEFAULT,
        None => COOKIE_THEME_DEFAULT,
    }
}

// ── SSE via fetch (POST + ReadableStream) ─────────────────────────────────────

async fn stream_chat(
    chat: ChatRequest,
    signal: AbortSignal,
    on_token: impl Fn(&str),
    on_total: impl Fn(u32),
) -> Result<(), String> {
    let win = web_sys::window().ok_or("no window")?; // TODO: thiserror

    let hdrs = web_sys::Headers::new().map_err(|e| format!("cannot create headers {e:?}"))?;
    hdrs.set("Content-Type", "application/json")
        .map_err(|e| format!("could not set content type to json : {e:?}"))?; // TODO: thiserror

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_headers(hdrs.as_ref());
    // TODO: use serde-wasm-bindgen = "0.6" ?
    // let js_val = serde_wasm_bindgen::to_value(&req)?;
    let body = serde_json::json!(chat).to_string();
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
    // pass the abord signal notifying end to the request
    // so that the browser/request can be cancelled from UI
    opts.set_signal(Some(&signal));

    let req = web_sys::Request::new_with_str_and_init("/api/chat", &opts)
        .map_err(|e| format!("could not create a new instance of request : {e:?}"))?; // TODO: thiserror

    let resp: web_sys::Response = JsFuture::from(win.fetch_with_request(&req))
        .await
        // TODO: which errors to handle ?
        .map_err(|e| format!("could not convert 1 : {e:?}"))?
        .dyn_into()
        // TODO: which errors to handle ?
        .map_err(|e| format!("could not convert 2 : {e:?}"))?;

    if !resp.ok() {
        // FIXME: this triggers when posting the chat fails
        // TODO: enhance and display for user
        let msg = format!("HTTP error: {} {}", resp.status(), resp.status_text(),);
        web_sys::console::error_1(&msg.clone().into());
        return Err(format!("HTTP {}", msg)); // TODO: thiserror
    }

    // this is a dumb wrapper around the browser JS reader, with no iterate, no await...
    let websys_reader = resp.body().ok_or("no body")?; // TODO: thiserror

    // wasm_streams wraps a JS reader into a proper futures::Stream<Item = Result<JsValue, JsValue>>
    // Internally it does the "JS stuff" : read(), extract "done" field via reflection,
    // because a JS chunk is JsValue {"done": false, "value": {"0": 100, "1": 97, ... }))
    // test if done, extract the value and provides it. It is converted from a concrete reader
    // into a future stream, so we can use iterator functions on it too.
    let wasm_stream = ReadableStream::from_raw(websys_reader).into_stream();

    // Convert each JsValue chunk value (which is a Uint8Array) into Bytes.
    let chunk_stream = wasm_stream.map(|chunk| {
        chunk
            // Network/server failure mid-stream, or AbortSignal fired
            .map_err(|e| format!("Error while reading chunks {e:?}")) // abort or network/server
            .map(|v| Uint8Array::new(&v).to_vec())
    });

    // decodes the spec-mandated utf-8, then manages line buffering
    // and parses SSE framing : make access easy to sse fields
    // must be mute because iterators are updated during loop
    let mut event_stream = chunk_stream.eventsource();

    // Iterate over the SSE events (easy-mode !)
    while let Some(result) = event_stream.next().await {
        match result {
            Ok(es_event) => {
                let sse_event = SseEventIn::try_from(es_event)
                    // FIXME: exiting the function, is it really the right choice ?
                    .map_err(|e| e.to_string())?
                    .into_inner();
                web_sys::console::debug_1(&format!("SSE event: {:?}", sse_event).into());
                match sse_event {
                    ChatEvent::TokenCount { prompt, generated } => {
                        // forward to the caller, who as access to the signals
                        on_total(prompt + generated);
                        web_sys::console::log_1(
                            &format!("Token count : prompt {prompt} answer={generated}",).into(),
                        );
                    }
                    ChatEvent::MessageToken(token) => {
                        on_token(&token);
                    }
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
            Err(e) => return Err(format!("SSE error: {e}")),
        }
    }
    Ok(())
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    // when a Rust panic occurs in WebAssembly, the browser just
    // shows a cryptic error like "unreachable executed" or similar.
    // installs a panic hook that intercepts panics and prints
    // the actual Rust panic message and stack trace to console
    console_error_panic_hook::set_once();

    // Restore theme from cookie BEFORE mounting to avoid any flash.
    // index.html already defaults to data-theme=Theme::Dark
    apply_theme(&get_cookie_theme_or_default());

    // takes root component (App) renders it into html body
    mount_to_body(App);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn save_chat(messages: &[Message]) {
    let Some(win) = window() else {
        // TODO: how to report error to user
        web_sys::console::warn_1(&"Save chat : no window available".into());
        return;
    };
    let storage = match win.session_storage() {
        Ok(Some(storage)) => storage,
        Ok(None) => return,
        Err(e) => {
            // TODO: how to report error to user
            web_sys::console::warn_1(&format!("Save chat : no storage available : {e:?}").into());
            return;
        }
    };
    let json = match serde_json::to_string(messages) {
        Ok(json) => json,
        Err(e) => {
            web_sys::console::warn_1(
                &format!("Save chat : error serializing chat for saving : {e:?}").into(),
            );
            return;
        }
    };
    // Ignore errors as browser privacy settings can disable storage
    if let Err(e) = storage.set_item(STORAGE_KEY_OPENAI, &json) {
        web_sys::console::warn_1(
            &format!("Saving chat failed, conversation is not persisted : {e:?}").into(),
        );
        // TODO: optionally display something in the UI
    }
}

fn load_chat() -> Vec<Message> {
    let Some(win) = window() else {
        // TODO: how to report error to user
        web_sys::console::warn_1(&"Load chat : no window available".into());
        return vec![];
    };
    let storage = match win.session_storage() {
        Ok(Some(storage)) => storage,
        Ok(None) => return vec![],
        Err(e) => {
            // TODO: how to report error to user
            web_sys::console::warn_1(&format!("Load chat : no storage available : {e:?}").into());
            return vec![];
        }
    };
    let text = match storage.get_item(STORAGE_KEY_OPENAI) {
        Ok(Some(text)) => text,
        Ok(None) => return vec![],
        Err(e) => {
            // TODO: how to report error to user
            web_sys::console::warn_1(
                &format!("Load chat : error retrieving saved chat : {e:?}").into(),
            );
            return vec![];
        }
    };

    match serde_json::from_str(&text) {
        Ok(chat) => chat,
        Err(e) => {
            web_sys::console::warn_1(
                &format!("Load chat : error deserializing saved chat : {e:?}").into(),
            );
            vec![]
        }
    }
}

fn show_error_alert(msg: &str) {
    let full = format!("Alert : {msg}");
    let Some(win) = web_sys::window() else {
        web_sys::console::warn_1(&"Show alert : no window available".into());
        web_sys::console::error_1(&full.into());
        return;
    };
    if let Err(e) = win.alert_with_message(&full) {
        web_sys::console::error_1(&format!("Show alert : alert failed : {e:?}").into());
        web_sys::console::error_1(&full.into());
    }
}

// ── App component ─────────────────────────────────────────────────────────────

#[component]
fn App() -> impl IntoView {
    // ── Signals ───────────────────────────────────────────────────────────────

    // the list of models available, compatible, and allowed for the config
    let models = RwSignal::new(vec![]);

    // used to get the currently selected model, if any
    let sel_model = RwSignal::new(String::new());

    // used to read the system prompt the user can modify
    let sys_prompt = RwSignal::new(String::new());

    // holds every conversation message, and receives the reply token in last one
    // stored to, and restored from, sessionStorage in case tab reloads
    let messages = RwSignal::new(load_chat());

    // used to interact with the input field for the user
    let input = RwSignal::new(String::new());

    // used to show/hide send/cancel buttons
    let streaming = RwSignal::new(false);

    // Check if we are restoring a saved conversation, which already started
    // - upon reload
    //   - if we restored a chat from storage,
    //     treat it as already started,
    //     so we don't re-inject the system prompt
    // - and if conversation is indeed started,
    //   - disables the system prompt
    //   - adds a system message if none were present
    //   - locks the model selection
    let started = RwSignal::new(!messages.get_untracked().is_empty());

    // used to allow anything in the UI to cancel ongoing requests
    // holds an abort controller which we can notify about cancelations
    let abort_ctl = RwSignal::new(None::<SendWrapper<AbortController>>);

    // used to update the UI (button icons) when the theme is changed (button clicked)
    let theme = RwSignal::new(get_cookie_theme_or_default());

    // a slot where the "conversation reference" will be stored
    let conv_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    // ── Bootstrap: load config then models ────────────────────────────────────
    spawn_local(async move {
        match Request::get("/api/config").send().await {
            Ok(r) => {
                if !r.ok() {
                    web_sys::console::error_1(&format!("HTTP error: {r:?}").into());
                    // TODO: handle HTTP errors better (display error, disable ui, etc)
                    return;
                }
                match r.json::<ConfigDto>().await {
                    Ok(cfg) => {
                        // load the predefined system prompt from config
                        sys_prompt.set(cfg.prepend_system_prompt);
                    }
                    Err(e) => {
                        show_error_alert(&format!("config parse failed: {e}"));
                    }
                }
            }
            Err(e) => {
                show_error_alert(&format!("config request failed: {e}"));
            }
        }
        match Request::get("/api/models").send().await {
            Ok(r) => {
                if !r.ok() {
                    web_sys::console::error_1(&format!("HTTP error: {r:?}").into());
                    // TODO: handle HTTP errors better (display error, disable ui, etc)
                    return;
                }
                match r.json::<Vec<ModelDto>>().await {
                    Ok(list) => {
                        // Try saved model cookie
                        if let Some(id) = get_cookie(COOKIE_MODEL)
                            .and_then(|s| list.iter().find(|m| m.id == s).map(|m| m.id.clone()))
                            // or fall back to first in list if absent or stale
                            .or_else(|| list.first().map(|m| m.id.clone()))
                        {
                            sel_model.set(id);
                        }
                        // Sets up selection dropdown
                        models.set(list);
                        // FIXME: handle empty list of models ... disable send button ?
                        // FIXME: started state should maybe have everything locked ?
                    }

                    Err(e) => {
                        show_error_alert(&format!("models parse failed: {e}"));
                    }
                }
            }
            Err(e) => {
                show_error_alert(&format!("models request failed: {e}"));
            }
        }
    });

    // ── Derived ───────────────────────────────────────────────────────────────

    // holds and updates metadata for the selected model
    let sel_meta = Memo::new(move |_| {
        let id = sel_model.get();
        // on a signal, .get() returns an owned *clone*,
        // so items could be moved out of it, if needed
        models.get().into_iter().find(|m| m.id == id)
    });

    // Writable signal, initialized to the default approximate(0)
    let tok_count = RwSignal::new(TokenUsage::default());

    // Effect replaces the Memo: tracks `messages`, so writes approximate
    // estimates EVERY TIME, but the exact over approx prio is done in
    // TokenUsage, so that is without consequence here
    Effect::new(move |_| {
        let approx = estimate_tokens(&messages.get());
        tok_count.update(|t| t.set_approximate(approx));
    });

    // allows locking model selection if started or no real "choice"
    // TODO: is it necessary to lock the selection if there is no real choice, i'd say no.
    let mdl_locked = Memo::new(move |_| started.get() || models.get().len() <= 1);

    // ── Auto-scroll on every new token ────────────────────────────────────────
    Effect::new(move |_| {
        // Leptos automatically "watches" what you ".get()" and monitors it for changes
        let _ = messages.get();
        // if we alread have a "conversation reference" (DOM element) to use
        if let Some(el) = conv_ref.get() {
            // scroll down, to the bottom of the div
            el.set_scroll_top(el.scroll_height());
        }
    });

    // ── Escape key stops streaming ────────────────────────────────────────────
    let esc = window_event_listener(leptos::ev::keydown, move |e: KeyboardEvent| {
        // if escape is pressed while streaming
        if e.key() == "Escape" && streaming.get_untracked() {
            // check if abort controller signal holds an abord controller
            if let Some(ac) = abort_ctl.get_untracked() {
                // trigger an abort (sig)
                ac.abort();
            }
            // clear streaming to update UI (hide stop, show send)
            streaming.set(false);
            // removes the abort controller from the abort controller signal
            abort_ctl.set(None);
        }
    });

    // esc is a WindowListenerHandle that removes the event listener when dropped
    // Without this, the keydown listener would outlive the component
    on_cleanup(move || drop(esc));

    // ── Theme toggle ──────────────────────────────────────────────────────────
    let toggle_theme = move |_| {
        let new_theme = match theme.get_untracked() {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        };
        apply_theme(&new_theme);
        set_cookie(COOKIE_THEME, new_theme.as_ref());
        theme.set(new_theme);
    };

    // ── Send ──────────────────────────────────────────────────────────────────
    let do_send = move || {
        if streaming.get_untracked() {
            // prevent multiple send
            return;
        }

        // IMPORTANT: handle fail path before user
        // This is the only way to link the browser (req) to the ui : 'ac' is
        // an abort-controller, linked to a abort-signal 'sig' created below
        let ac = match AbortController::new() {
            Ok(ac) => ac,
            Err(e) => {
                // surface to user or log; abort-less streaming is still better than nothing
                web_sys::console::error_1(
                    &format!("Failed to create abort-controller: {e:?}").into(),
                );
                // TODO: how to report error to user
                // TODO: optionally: fall back to streaming without abort support ?
                return;
            }
        };

        // get user input
        let text = input.get_untracked().trim().to_string();
        if text.is_empty() {
            return;
        }

        // get selected model name
        let model = sel_model.get_untracked();
        if model.is_empty() {
            show_error_alert("No model selected — the model list may have failed to load.");
            return;
        }

        // get whole history (stored across reloads, thrown away on tab discard)
        let mut hist = messages.get_untracked();

        if !started.get_untracked() {
            // Always send a system message on first turn — even if empty.
            // An empty system message signals the backend to skip its own injection.
            hist.insert(
                0,
                Message {
                    role: MessageRole::System,
                    content: sys_prompt.get_untracked(),
                },
            );
            started.set(true);
        }

        // Add the input message to the history
        hist.push(Message {
            role: MessageRole::User,
            content: text,
        });

        // Add an empty message and add it to the list : this is a reserved slot
        // to later accumulate incoming token during the streaming reply :
        // - the streaming closure captures messages
        // - appends tokens to messages.last_mut()
        // - the empty slot must exist in the signal before streaming begins
        //   or the first token has nowhere to land
        hist.push(Message {
            role: MessageRole::Assistant,
            content: String::new(),
        });

        // Do not send the last empty assistant slot
        // FIXME: seems suboptimal, play with it !
        let send_msgs = hist[..hist.len() - 1].to_vec();

        // Persist the history (except last empty) to sessionStorage (in case tab is reloaded)
        // FIXME: model is not saved s?!
        save_chat(&send_msgs);

        // Moves message list to Leptos, which stores the value in a reference-counted cell
        // on the reactive heap, kept alive as long as any signal handle or subscriber
        // references it. On the UI side, this triggers every reactive subscriber that
        // called .get() on messages — specifically the conversation renderer and tok_count memo.
        // Leptos diffs the new value and patches the DOM on every value (actual) change
        messages.set(hist);

        // Clear the input so the user can prepare his next message while streaming
        input.set(String::new());

        // IMPORTANT: this should be positionned AFTER all the "pre-request"  fail paths
        // have been resolved, so that this is not set with no way to recover !
        // Set the UI to streaming state (shows cancel button instead of send button)
        streaming.set(true);

        // Now the non-failing parts of the abort controller 'ac' duo : this
        // 'sig' will be passed/moved into the promise so it could notify it
        let sig = ac.signal();
        // 'ac' is stored in a rwsignal, so that the UI could abord the req
        abort_ctl.set(Some(SendWrapper::new(ac)));

        // Builds the actual JSON payload to send to the server
        let chat_req = ChatRequest {
            model,
            messages: send_msgs,
        };

        // Launch an additional async task, which will stream and update, and let it run freely
        spawn_local(async move {
            // do the work, providing a closure to handle each new token
            let res = stream_chat(
                chat_req,
                sig,
                move |tok| {
                    // update the message list by
                    messages.update(|v| {
                        // looking for the last one (that is why we added an empty one)
                        if let Some(last) = v.last_mut() {
                            // and appending the newest token to its content
                            last.content.push_str(&tok);
                        }
                    });
                },
                move |new_total| {
                    // update counter, but only if changed: we have no Memo to
                    // do the dedup. Not necessary, but it is to practice.
                    if tok_count.get_untracked() != TokenUsage::Exact(new_total) {
                        tok_count.update(|t| t.set_exact(new_total));
                    }
                },
            )
            // run until completion
            .await;

            // we got a complete response (either successfully or not)
            streaming.set(false);

            // removes the abort controller from the abort controller signal
            abort_ctl.set(None);

            // If we had any error while sending the chat
            if let Err(e) = res {
                web_sys::console::error_1(&format!("Chat error: {e}").into());
                // TODO: show error in the UI, but not in the specific bubble ?
                // TODO: thiserror instead of string handling ?
                let e_low = e.to_lowercase();
                if !e_low.contains("abort") && !e_low.contains("cancel") {
                    // TODO: maybe move to a dedicated error notification area ?
                    messages.update(|v| {
                        // last message (whatever it is, user or assistant)
                        // but due to the workflow, the assistant
                        // gets the error message, for the user to read
                        if let Some(last) = v.last_mut() {
                            last.content = format!("⚠ Error: {e}");
                        }
                    });
                }
            } else {
                // only save reply to sessionStorage upon success
                save_chat(&messages.get_untracked());
            }
        });
    };

    // ── Stop ──────────────────────────────────────────────────────────────────
    let do_stop = move || {
        // if we have an abort-controller, notify it
        if let Some(ac) = abort_ctl.get_untracked() {
            ac.abort();
        }
        // revert the UI back to normal
        streaming.set(false);
        // removes the abort controller from the abort controller signal
        abort_ctl.set(None);
    };

    // ── View ──────────────────────────────────────────────────────────────────
    view! {
        <div class="app">

            // ── Banner ────────────────────────────────────────────────────────
            <div class="banner">

                // ── Left-most: clear session + reload ──
                <button
                    class="btn-clear"
                    title="Clear conversation and reload"
                    on:click=move |_| {
                        // Abort any in-flight request cleanly
                        do_stop();

                        // Erase the chat from browser storage
                        save_chat(&vec![]);

                        // Reload the tab
                        match web_sys::window() {
                            Some(window) => match window.location().reload() {
                                Ok(_) => {}
                                Err(e) => {
                                    web_sys::console::error_1(
                                        &format!("Could not reload current window : {e:?}").into(),
                                    );
                                }
                            },
                            None => {}
                        }
                    }
                >
                    "✕"
                </button>

                // Left: model dropdown
                <select
                    class="model-select"
                    prop:value=move || sel_model.get()
                    prop:disabled=move || mdl_locked.get()
                    on:change=move |e| {
                        let val = event_target_value(&e);
                        // notify the rest of the UI hat model changed
                        sel_model.set(val.clone());
                        // persist to cookie
                        set_cookie(COOKIE_MODEL, &val);
                    }
                >
                    {move || models.get().into_iter().map(|m| {
                        let id = m.id.clone();
                        view! { <option value=id.clone()>{id.clone()}</option> }
                    }).collect_view()}
                </select>

                // Center: token counter (color is dynamic, layout is in CSS)
                <div style="flex:1; display:flex; justify-content:center;">
                    <span
                        class="token-counter"
                        style=move || token_color_style(
                            &tok_count.get(),
                            sel_meta.get().and_then(|m| m.context_window),
                        )
                    >
                        {move || match sel_meta.get().and_then(|m| m.context_window) {
                            // TokenUsae handles the '~' (or not) for variants
                            Some(max) => format!("{} token / {}k", tok_count.get(), max / 1024),
                            None      => format!("{} token", tok_count.get()),
                        }}
                    </span>
                </div>

                // Right: title link + theme toggle
                <div class="banner-right">
                    <a href="https://github.com/nipil/openai-compatible-chat" class="github-link" target="_blank">
                        <i class="fab fa-github"></i>
                    </a>
                    <button class="theme-btn" on:click=toggle_theme>
                        {move || match theme.get() { Theme::Dark => "🌞" , Theme::Light => "🌚" }}
                    </button>
                </div>


                // ── Right-most: open same URL in a new tab ──
                <button
                    class="btn-new"
                    title="Open a new conversation tab"
                    on:click=move |_| {
                        let Some(window) = web_sys::window() else {
                            web_sys::console::error_1(&"No window available".into());
                            return;
                        };
                        let Ok(href) = window.location().href() else {
                            web_sys::console::error_1(&"Could not read current location".into());
                            return;
                        };
                        if let Err(e) = window.open_with_url_and_target(&href, "_blank") {
                            web_sys::console::error_1(&format!("Could not open new windo : {e:?}").into());
                            return;
                        }
                    }
                >
                    "＋"
                </button>
            </div>

            // ── System prompt ─────────────────────────────────────────────────
            <div class="sys-area">
                <textarea
                    class="sys-textarea"
                    placeholder="System prompt — editable until first message is sent…"
                    prop:value=move || sys_prompt.get()
                    prop:disabled=move || started.get()
                    on:input=move |e| sys_prompt.set(event_target_value(&e))
                    rows="2"
                />
            </div>

            // ── Conversation ──────────────────────────────────────────────────

            // on first render, Leptos stores a reference to this DOM element
            // (aka a node_ref) in conv_ref for later reference/use
            <div class="conversation" node_ref=conv_ref>
                // populates the UI with the app DOM elements every
                // time messages (tracked by .get()) is updated
                {move || messages.get().into_iter()
                    .filter(|m| m.role != MessageRole::System)
                    .map(|msg| {
                        let row_cls = format!("msg-row {}", msg.role);
                        let bubble_cls = format!("msg-bubble {}", msg.role);
                        view! {
                            <div class=row_cls>
                                <div class=bubble_cls inner_html=to_html(&msg.content) />
                            </div>
                        }
                    })
                    .collect_view()
                }
            </div>

            // ── Input area ────────────────────────────────────────────────────
            <div class="input-area">
                <textarea
                    class="input-textarea"
                    placeholder="Message… (Ctrl+Enter to send  •  Esc to stop)"
                    prop:value=move || input.get()
                    on:input=move |e| input.set(event_target_value(&e))
                    on:keydown=move |e: KeyboardEvent| {
                        if e.ctrl_key() && e.key() == "Enter" && !streaming.get_untracked() {
                            // prevent multiple send ?
                            do_send();
                        }
                    }
                    rows="3"
                />
                {move || if streaming.get() {
                    view! {
                        <button class="btn-stop" on:click=move |_| do_stop()>
                            "⏹ Stop (Esc)"
                        </button>
                    }.into_any()
                } else {
                    view! {
                        <button class="btn-send" on:click=move |_| do_send()>
                            "Send ↵"
                        </button>
                    }.into_any()
                }}
            </div>
        </div>
    }
}
