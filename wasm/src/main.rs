use gloo_net::http::Request;
use leptos::{mount::mount_to_body, prelude::*, task::spawn_local};
use send_wrapper::SendWrapper;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortController, AbortSignal, KeyboardEvent, ReadableStreamDefaultReader};

// ── DTOs (mirror backend) ─────────────────────────────────────────────────────

// PartialEq required by a bound in `leptos::prelude::Memo::<T>::new`
#[derive(Clone, PartialEq, Debug, Deserialize)]
struct ModelDto {
    id: String,
    max_tokens: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
struct ConfigDto {
    system_prompt: String,
    locked_model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

// ── Token estimate (mirrors backend logic) ────────────────────────────────────

fn estimate(msgs: &[Message]) -> usize {
    msgs.iter()
        .map(|m| 3 + m.content.chars().count() / 4)
        .sum::<usize>()
        + 3
}

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

fn apply_theme(dark: bool) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(element) = document.document_element() else {
        return;
    };
    let _ = element.set_attribute("data-theme", if dark { "dark" } else { "light" });
}

// ── SSE via fetch (POST + ReadableStream) ─────────────────────────────────────

async fn stream_chat(
    body: String,
    signal: AbortSignal,
    on_token: impl Fn(String),
) -> Result<(), String> {
    let win = web_sys::window().ok_or("no window")?;

    let hdrs = web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    hdrs.set("Content-Type", "application/json")
        .map_err(|e| format!("{e:?}"))?;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_headers(hdrs.as_ref());
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
    opts.set_signal(Some(&signal));

    let req = web_sys::Request::new_with_str_and_init("/api/chat", &opts)
        .map_err(|e| format!("{e:?}"))?;

    let resp: web_sys::Response = JsFuture::from(win.fetch_with_request(&req))
        .await
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        .map_err(|e| format!("{e:?}"))?;

    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let reader: ReadableStreamDefaultReader = resp
        .body()
        .ok_or("no body")?
        .get_reader()
        .dyn_into()
        .map_err(|e| format!("{e:?}"))?;

    let mut buf = String::new();

    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|e| format!("{e:?}"))?;

        let done = js_sys::Reflect::get(&chunk, &"done".into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if done {
            break;
        }

        let value = js_sys::Reflect::get(&chunk, &"value".into()).map_err(|e| format!("{e:?}"))?;
        let arr: js_sys::Uint8Array = value.dyn_into().map_err(|e| format!("{e:?}"))?;
        buf.push_str(&String::from_utf8_lossy(&arr.to_vec()));

        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim_end_matches('\r').to_string();
            buf = buf[nl + 1..].to_string();
            if let Some(data) = line.strip_prefix("data: ") {
                if data != "[DONE]" {
                    on_token(data.to_string());
                }
            }
        }
    }
    Ok(())
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    console_error_panic_hook::set_once();
    // Restore theme from cookie BEFORE mounting to avoid any flash.
    // index.html already defaults to data-theme="dark"; this only fires
    // when the user has previously saved "light".
    let dark = get_cookie("theme").map(|v| v != "light").unwrap_or(true);
    apply_theme(dark);
    mount_to_body(App);
}

// ── App component ─────────────────────────────────────────────────────────────

#[component]
fn App() -> impl IntoView {
    // ── Signals ───────────────────────────────────────────────────────────────
    let models = RwSignal::new(vec![]);
    let sel_model = RwSignal::new(String::new());
    let locked_mdl = RwSignal::new(None::<String>);
    let sys_prompt = RwSignal::new(String::new());
    let messages = RwSignal::new(vec![]);
    let input = RwSignal::new(String::new());
    let streaming = RwSignal::new(false);
    let started = RwSignal::new(false);
    let abort_ctl = RwSignal::new(None::<SendWrapper<AbortController>>);
    let is_dark = RwSignal::new(get_cookie("theme").map(|v| v != "light").unwrap_or(true));
    let conv_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    // ── Bootstrap: load config then models ────────────────────────────────────
    spawn_local(async move {
        if let Ok(r) = Request::get("/api/config").send().await {
            if let Ok(cfg) = r.json::<ConfigDto>().await {
                sys_prompt.set(cfg.system_prompt);
                locked_mdl.set(cfg.locked_model);
            }
        }
        if let Ok(r) = Request::get("/api/models").send().await {
            if let Ok(list) = r.json::<Vec<ModelDto>>().await {
                let locked = locked_mdl.get_untracked();
                let initial = if locked.is_none() {
                    // Try saved model cookie; fall back to first in list if absent/stale
                    get_cookie("model")
                        .and_then(|s| list.iter().find(|m| m.id == s).map(|m| m.id.clone()))
                        .or_else(|| list.first().map(|m| m.id.clone()))
                } else {
                    // Locked model: cookie is irrelevant, take the only option
                    list.first().map(|m| m.id.clone())
                };
                if let Some(id) = initial {
                    sel_model.set(id);
                }
                models.set(list);
            }
        }
    });

    // ── Derived ───────────────────────────────────────────────────────────────
    let sel_meta = Memo::new(move |_| {
        let id = sel_model.get();
        models.get().into_iter().find(|m| m.id == id)
    });
    let tok_count = Memo::new(move |_| estimate(&messages.get()));
    let mdl_locked =
        Memo::new(move |_| started.get() || locked_mdl.get().is_some() || models.get().len() <= 1);

    // ── Auto-scroll on every new token ────────────────────────────────────────
    Effect::new(move |_| {
        let _ = messages.get();
        if let Some(el) = conv_ref.get() {
            el.set_scroll_top(el.scroll_height());
        }
    });

    // ── Escape key stops streaming ────────────────────────────────────────────
    let esc = window_event_listener(leptos::ev::keydown, move |e: KeyboardEvent| {
        if e.key() == "Escape" && streaming.get_untracked() {
            if let Some(ac) = abort_ctl.get_untracked() {
                ac.abort();
            }
            streaming.set(false);
            abort_ctl.set(None);
        }
    });
    on_cleanup(move || drop(esc));

    // ── Theme toggle ──────────────────────────────────────────────────────────
    let toggle_theme = move |_| {
        let new_dark = !is_dark.get_untracked();
        is_dark.set(new_dark);
        apply_theme(new_dark);
        set_cookie("theme", if new_dark { "dark" } else { "light" });
    };

    // ── Send ──────────────────────────────────────────────────────────────────
    let do_send = move || {
        if streaming.get_untracked() {
            return;
        }
        let text = input.get_untracked().trim().to_string();
        if text.is_empty() {
            return;
        }

        let model = locked_mdl
            .get_untracked()
            .unwrap_or_else(|| sel_model.get_untracked());

        let mut hist = messages.get_untracked();

        if !started.get_untracked() {
            // Always send a system message on first turn — even if empty.
            // An empty system message signals the backend to skip its own injection.
            hist.insert(
                0,
                Message {
                    role: "system".into(),
                    content: sys_prompt.get_untracked(),
                },
            );
            started.set(true);
        }

        hist.push(Message {
            role: "user".into(),
            content: text,
        });
        hist.push(Message {
            role: "assistant".into(),
            content: String::new(),
        }); // reserved slot
        let send_msgs = hist[..hist.len() - 1].to_vec(); // exclude the empty assistant slot
        messages.set(hist);
        input.set(String::new());
        streaming.set(true);

        let ac = AbortController::new().unwrap();
        let sig = ac.signal();
        abort_ctl.set(Some(SendWrapper::new(ac)));

        let body = serde_json::json!({ "model": model, "messages": send_msgs }).to_string();

        spawn_local(async move {
            let res = stream_chat(body, sig, move |tok| {
                messages.update(|v| {
                    if let Some(last) = v.last_mut() {
                        last.content.push_str(&tok);
                    }
                });
            })
            .await;

            streaming.set(false);
            abort_ctl.set(None);

            if let Err(e) = res {
                let e_low = e.to_lowercase();
                if !e_low.contains("abort") && !e_low.contains("cancel") {
                    messages.update(|v| {
                        if let Some(last) = v.last_mut() {
                            last.content = format!("⚠ Error: {e}");
                        }
                    });
                }
            }
        });
    };

    // ── Stop ──────────────────────────────────────────────────────────────────
    let do_stop = move || {
        if let Some(ac) = abort_ctl.get_untracked() {
            ac.abort();
        }
        streaming.set(false);
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
                        sel_model.set(val.clone());
                        // Only persist to cookie when the user freely chose the model
                        if locked_mdl.get_untracked().is_none() {
                            set_cookie("model", &val);
                        }
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
                            sel_meta.get().and_then(|m| m.max_tokens),
                        )
                    >
                        {move || match sel_meta.get().and_then(|m| m.max_tokens) {
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
                        {move || if is_dark.get() { "🌞" } else { "🌚" }}
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
            <div class="conversation" node_ref=conv_ref>
                {move || messages.get().into_iter()
                    .filter(|m| m.role != "system")
                    .map(|msg| {
                        let is_user    = msg.role == "user";
                        let row_cls    = if is_user { "msg-row user" }      else { "msg-row assistant" };
                        let bubble_cls = if is_user { "msg-bubble user" }   else { "msg-bubble assistant" };
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
