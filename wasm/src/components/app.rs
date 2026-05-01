use leptos::prelude::*;
use leptos::task::spawn_local;
use portable::{
    ChatRequest, ConfigDto, Message, MessageRole, ModelDto, Theme, TokenUsage, estimate_tokens,
};
use send_wrapper::SendWrapper;
use web_sys::{AbortController, KeyboardEvent};

use super::banner::Banner;
use super::conversation::Conversation;
use super::error_panel::ErrorPanel;
use super::input_area::InputArea;
use super::system_prompt::SystemPrompt;
use crate::api::client::get_url_path;
use crate::api::sse::stream_chat;
use crate::utils::keyboard::KeyboardId;
use crate::utils::storage::{load_chat, save_chat};
use crate::web::cookies::{COOKIE_MODEL, COOKIE_THEME, get_cookie, get_cookie_theme, set_cookie};
use crate::web::dom::get_window;
use crate::web::theme::apply_theme;
use crate::{AppError, BrowserError, LogicError, handle_err, handle_err_clos_1, handle_err_fut_0};

#[component]
pub fn App() -> impl IntoView {
    // ── Signals ────────────────────────────────────────────────────────────────

    let errors: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    let models: RwSignal<Vec<ModelDto>> = RwSignal::new(vec![]);
    let sel_model = RwSignal::new(String::new());
    let sys_prompt = RwSignal::new(String::new());
    let messages = RwSignal::new(handle_err(errors, load_chat().map_err(|e| e.into())));
    let input = RwSignal::new(String::new());
    let streaming = RwSignal::new(false);
    let started = RwSignal::new(!messages.get_untracked().is_empty());
    let abort_ctl = RwSignal::new(None::<SendWrapper<AbortController>>);
    let theme = RwSignal::new(get_cookie_theme());
    let conv_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    // ── Bootstrap ─────────────────────────────────────────────────────────────

    spawn_local(handle_err_fut_0(errors, async move {
        let cfg = get_url_path::<ConfigDto>("/api/config").await?;
        sys_prompt.set(cfg.default_system_prompt);
        let mut list = get_url_path::<Vec<ModelDto>>("/api/models").await?;
        list.sort();
        if let Ok(Some(id)) = get_cookie(COOKIE_MODEL)
            && let Some(found) = list.iter().find(|m| m.id == id)
        {
            sel_model.set(found.id.clone());
        } else if !list.is_empty() {
            sel_model.set(list[0].id.clone());
        }
        models.set(list);
        Ok(())
    }));

    // ── Derived ────────────────────────────────────────────────────────────────

    let sel_meta = Memo::new(move |_| {
        let id = sel_model.get();
        models.get().into_iter().find(|m| m.id == id)
    });

    let tok_count = RwSignal::new(TokenUsage::default());

    Effect::new(move |_| {
        let approx = estimate_tokens(&messages.get());
        tok_count.update(|t| t.set_approximate(approx));
    });

    let mdl_locked = Memo::new(move |_| started.get() || models.get().len() <= 1);

    // ── Auto-scroll ───────────────────────────────────────────────────────────

    Effect::new(move |_| {
        let _ = messages.get();
        if let Some(el) = conv_ref.get() {
            el.set_scroll_top(el.scroll_height());
        }
    });

    // ── Escape key ────────────────────────────────────────────────────────────

    let esc = window_event_listener(leptos::ev::keydown, move |e: KeyboardEvent| {
        if e.key() == KeyboardId::Escape.as_ref() && streaming.get_untracked() {
            if let Some(ac) = abort_ctl.get_untracked() {
                ac.abort();
            }
            streaming.set(false);
            abort_ctl.set(None);
        }
    });
    on_cleanup(move || drop(esc));

    // ── Stop ──────────────────────────────────────────────────────────────────

    let do_stop = move || {
        if let Some(ac) = abort_ctl.get_untracked() {
            ac.abort();
        }
        streaming.set(false);
        abort_ctl.set(None);
    };

    // ── Send ──────────────────────────────────────────────────────────────────

    let do_send = move || -> Result<(), AppError> {
        if streaming.get_untracked() {
            web_sys::console::debug_1(&"Prevent multiple send".into());
            return Ok(());
        }

        let ac = AbortController::new()
            .map_err(|e| BrowserError::AbortController { source: e.into() })?;

        let text = input.get_untracked().trim().to_string();
        if text.is_empty() {
            web_sys::console::debug_1(&"Empty input, ignoring".into());
            return Ok(());
        }

        let model = sel_model.get_untracked();
        if model.is_empty() {
            return Err(LogicError::NoModelSelected.into());
        }

        let mut hist = messages.get_untracked();

        if !started.get_untracked() {
            hist.insert(
                0,
                Message::new(MessageRole::System, sys_prompt.get_untracked()),
            );
            started.set(true);
        }

        hist.push(Message::new(MessageRole::User, text));
        let send_msgs = hist.clone();
        hist.push(Message::new(MessageRole::Assistant, String::new()));

        save_chat(&send_msgs)?;
        messages.set(hist);
        input.set(String::new());
        streaming.set(true);

        let abort_signal = ac.signal();
        abort_ctl.set(Some(SendWrapper::new(ac)));

        let chat_req = ChatRequest::new(model, send_msgs);

        let win = crate::web::dom::get_window()?;

        spawn_local(handle_err_fut_0(errors, async move {
            let res = stream_chat(
                win,
                chat_req,
                abort_signal,
                move |tok| {
                    messages.update(|v| {
                        if let Some(last) = v.last_mut() {
                            last.content.push_str(tok);
                        }
                    });
                },
                move |new_total| {
                    if tok_count.get_untracked() != TokenUsage::Exact(new_total) {
                        tok_count.update(|t| t.set_exact(new_total));
                    }
                },
            )
            .await;

            streaming.set(false);
            abort_ctl.set(None);

            if res.is_err() {
                messages.update(|mv| {
                    mv.pop_if(|m| m.role == MessageRole::Assistant);
                });
                messages.update(|mv| {
                    if let Some(message) = mv.pop_if(|m| m.role == MessageRole::User) {
                        input.set(message.content);
                    }
                });
            }

            res?;
            save_chat(&messages.get_untracked())?;
            Ok(())
        }));

        Ok(())
    };

    // ── Callbacks for Banner ─────────────────────────────────────────────────

    let toggle_theme = handle_err_clos_1(errors, move |_| {
        let new_theme = match theme.get_untracked() {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        };
        apply_theme(&new_theme)?;
        set_cookie(COOKIE_THEME, new_theme.as_ref())?;
        theme.set(new_theme);
        Ok(())
    });

    let open_new_tab = handle_err_clos_1(errors, move |_| {
        let window = get_window()?;
        let href = window
            .location()
            .href()
            .map_err(|e| BrowserError::CurrentUrl { source: e.into() })?;
        window
            .open_with_url_and_target(&href, "_blank")
            .map_err(|e| BrowserError::OpenWindow { source: e.into() })?;
        Ok(())
    });

    let on_clear = handle_err_clos_1(errors, move |_| {
        do_stop();
        save_chat(&[])?;
        get_window()?
            .location()
            .reload()
            .map_err(|e| BrowserError::ReloadFailed { source: e.into() }.into())
    });

    // ── View ───────────────────────────────────────────────────────────────────

    view! {
        <div class="app">
            <Banner
                errors=errors
                models=models
                sel_model=sel_model
                mdl_locked=mdl_locked
                tok_count=tok_count
                sel_meta=sel_meta
                theme=theme
                on_clear=on_clear
                on_new_tab=open_new_tab
                on_toggle_theme=toggle_theme
            />
            <SystemPrompt sys_prompt=sys_prompt started=started />
            <Conversation messages=messages conv_ref=conv_ref />
            <ErrorPanel errors=errors />
            <InputArea
                errors=errors
                input=input
                streaming=streaming
                on_send=do_send
                on_stop=do_stop
            />
        </div>
    }
}
