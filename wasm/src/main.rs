use gloo_net::http::Request;
use leptos::{mount::mount_to_body, prelude::*, task::spawn_local};
use portable::{ConfigDto, Message, MessageRole, ModelDto, Theme, estimate_tokens};
use send_wrapper::SendWrapper;
use std::str::FromStr;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortController, AbortSignal, KeyboardEvent, ReadableStreamDefaultReader, window};

const COOKIE_MODEL: &str = "model";
const COOKIE_THEME_DEFAULT: Theme = Theme::Dark;
const COOKIE_THEME: &str = "theme";
const STORAGE_KEY_OPENAI: &str = "openai";

// Token counter: returns an inline style string for the dynamic gradient only.
// Static layout/padding lives in .token-counter in style.css.
fn token_color_style(count: usize, max: Option<u32>) -> String {
    let (bg, color) = match max {
        None => ("rgba(128,128,128,0.4)", "var(--text-banner)"),
        Some(m) => {
            let pct = count as f64 / m as f64;
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
    body: String,
    signal: AbortSignal,
    on_token: impl Fn(String),
) -> Result<(), String> {
    let win = web_sys::window().ok_or("no window")?; // TODO: thiserror

    let hdrs = web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    hdrs.set("Content-Type", "application/json")
        .map_err(|e| format!("{e:?}"))?; // TODO: thiserror

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_headers(hdrs.as_ref());
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
    // pass the abord signal notifying end to the request
    // so that the browser/request can be cancelled from UI
    opts.set_signal(Some(&signal));

    let req = web_sys::Request::new_with_str_and_init("/api/chat", &opts)
        .map_err(|e| format!("{e:?}"))?; // TODO: thiserror

    let resp: web_sys::Response = JsFuture::from(win.fetch_with_request(&req))
        .await
        // TODO: which errors to handle ?
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        // TODO: which errors to handle ?
        .map_err(|e| format!("{e:?}"))?;

    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status())); // TODO: thiserror
    }

    let reader: ReadableStreamDefaultReader = resp
        .body()
        .ok_or("no body")? // TODO: thiserror
        .get_reader()
        .dyn_into()
        .map_err(|e| format!("{e:?}"))?; // TODO: thiserror

    let mut buf = String::new();

    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            // TODO: which errors to handle ?
            .map_err(|e| format!("{e:?}"))?;

        // TODO: done string to enum
        // TODO: report to user?
        let done = js_sys::Reflect::get(&chunk, &"done".into())
            .map_err(|e| format!("could not read 'done' from stream chunk : {e:?}"))?
            .as_bool()
            .ok_or_else(|| "stream chunk 'done' is not a boolean".to_string())?;

        if done {
            break;
        }

        let value = js_sys::Reflect::get(&chunk, &"value".into()) // TODO: to enum https://developer.mozilla.org/en-US/docs/Web/API/ReadableStreamDefaultReader/read#return_value
            .map_err(|e| format!("{e:?}"))?; // TODO: thiserror
        let arr: js_sys::Uint8Array = value.dyn_into().map_err(|e| format!("{e:?}"))?; // TODO: thiserror

        // The SSE spec guarantees UTF-8, so lossy is technically wrong here
        // But a replacement character \u{FFFD} in the stream would corrupt
        // rendered markdown silently, so this
        // TODO: try as non-lossless UTF-8, and if it fails, switch to lossy
        // TODO: then add a display warning marking the message as lossy
        buf.push_str(&String::from_utf8_lossy(&arr.to_vec()));

        // processes all complete lines from the buffer in one chunk,
        // since a single network chunk may contain multiple \n-terminated SSE frames

        // TODO: does not handle other line endings according to spec (simple \r)
        // How this does work : does a manual ring-buffer drain because :
        // - a single TCP chunk can contain multiple SSE frames,
        // - and frames can also be split across chunks
        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim_end_matches('\r').to_string();
            buf = buf[nl + 1..].to_string();
            // TODO: the space after the colon is not mandatory, but 1 space is stripped if present
            if let Some(data) = line.strip_prefix("data: ") {
                // this is openai stuff, not SSE spec
                if data != "[DONE]" {
                    // decode the token from json so that newlines in the token
                    // were not lost in the SSE frame, and are preserved for frontend
                    // and if not decodable, use as it
                    // TODO: notify user ?s
                    let token: String = serde_json::from_str(data).unwrap_or_else(|e| {
                        web_sys::console::warn_1(&format!("token parse failed: {e}").into());
                        // we choose to NOT abort, and juste provide it "as-is"
                        data.to_string()
                    });
                    #[cfg(feature = "print-tokens")]
                    web_sys::console::log_1(&format!("token: {:?}", token).into());
                    // call the callback for each token
                    on_token(token);
                }
            }
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
    web_sys::console::error_1(&format!("Alert : {msg}").into());
    match web_sys::window() {
        Some(win) => {
            if let Err(e) = win.alert_with_message(msg) {
                web_sys::console::error_1(&format!("Show alert : alert failed : {e:?}").into());
            }
        }
        None => {
            web_sys::console::warn_1(&"Show alert : no window available".into());
        }
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
            Ok(r) => match r.json::<ConfigDto>().await {
                Ok(cfg) => {
                    // load the predefined system prompt from config
                    sys_prompt.set(cfg.prepend_system_prompt);
                }
                Err(e) => {
                    show_error_alert(&format!("config parse failed: {e}"));
                }
            },
            Err(e) => {
                show_error_alert(&format!("config request failed: {e}"));
            }
        }
        match Request::get("/api/models").send().await {
            Ok(r) => match r.json::<Vec<ModelDto>>().await {
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
            },
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

    // updates tok_count every time messages change
    let tok_count = Memo::new(move |_| estimate_tokens(&messages.get()));

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
                web_sys::console::error_1(&e);
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
        save_chat(&send_msgs);

        // Now when the , the display will update
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
        // TODO: serde struct, and move it
        let body = serde_json::json!({ "model": model, "messages": send_msgs }).to_string();

        // Launch an additional async task, which will stream and update, and let it run freely
        spawn_local(async move {
            // do the work, providing a closure to handle each new token
            let res = stream_chat(body, sig, move |tok| {
                // update the message list by
                messages.update(|v| {
                    // looking for the last one (that is why we added an empty one)
                    if let Some(last) = v.last_mut() {
                        // and appending the newest token to its content
                        last.content.push_str(&tok);
                    }
                });
            })
            // run until completion
            .await;

            // we got a complete response (either successfully or not)
            streaming.set(false);

            // removes the abort controller from the abort controller signal
            abort_ctl.set(None);

            // If we had any error while sending the chat
            if let Err(e) = res {
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
                save_chat(&messages.get()); // only save reply to sessionStorage upon success
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
                            tok_count.get(),
                            sel_meta.get().and_then(|m| m.context_window),
                        )
                    >
                        {move || match sel_meta.get().and_then(|m| m.context_window) {
                            Some(max) => format!("~{} / {} tok", tok_count.get(), max),
                            None      => format!("~{} tok", tok_count.get()),
                        }}
                    </span>
                </div>

                // Right: title link + theme toggle
                <div class="banner-right">
                    <a
                        class="title-link"
                        href="https://github.com/nipil/openai-compatible-chat"
                        target="_blank"
                    >
                        "openai-compatible-chat"
                    </a>
                    <button class="theme-btn" on:click=toggle_theme>
                        {move || match theme.get() { Theme::Dark => "🌞" , Theme::Light => "🌚" }}
                    </button>
                </div>
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
