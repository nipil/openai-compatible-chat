use leptos::prelude::*;
use portable::{ModelDto, Theme, TokenUsage};

use crate::handle_err_clos_1;
use crate::utils::tokens::token_color_style;
use crate::web::cookies::{COOKIE_MODEL, set_cookie};

#[component]
pub fn Banner(
    errors: RwSignal<Vec<String>>,
    models: RwSignal<Vec<ModelDto>>,
    sel_model: RwSignal<String>,
    mdl_locked: Memo<bool>,
    tok_count: RwSignal<TokenUsage>,
    sel_meta: Memo<Option<ModelDto>>,
    theme: RwSignal<Theme>,
    on_clear: impl Fn(web_sys::MouseEvent) + 'static,
    on_new_tab: impl Fn(web_sys::MouseEvent) + 'static,
    on_toggle_theme: impl Fn(web_sys::MouseEvent) + 'static,
) -> impl IntoView {
    view! {
        <div class="banner">

            <button class="btn-clear" title="Clear conversation and reload" on:click=on_clear>
                "✕"
            </button>

            <select
                class="model-select"
                prop:value=move || sel_model.get()
                prop:disabled=move || mdl_locked.get()
                on:change=handle_err_clos_1(errors, move |e| {
                    let model_id = event_target_value(&e);
                    sel_model.set(model_id.clone());
                    set_cookie(COOKIE_MODEL, &model_id).map_err(|e| e.into())
                })
            >
                {move || models.get().into_iter().map(|m| {
                    let id = m.id.clone();
                    view! { <option value=id.clone()>{id.clone()}</option> }
                }).collect_view()}
            </select>

            <div style="flex:1; display:flex; justify-content:center;">
                <span
                    class="token-counter"
                    style=move || token_color_style(
                        &tok_count.get(),
                        sel_meta.get().and_then(|m| m.context_window),
                    )
                >
                    {move || match sel_meta.get().and_then(|m| m.context_window) {
                        Some(max) => format!("{} token / {}k", tok_count.get(), max / 1024),
                        None      => format!("{} token", tok_count.get()),
                    }}
                </span>
            </div>

            <div class="banner-right">
                <a
                    href="https://github.com/nipil/openai-compatible-chat"
                    class="github-link"
                    target="_blank"
                >
                    <i class="fab fa-github"></i>
                </a>
                <button class="theme-btn" on:click=on_toggle_theme>
                    {move || match theme.get() { Theme::Dark => "🌞", Theme::Light => "🌚" }}
                </button>
            </div>

            <button class="btn-new" title="Open a new conversation tab" on:click=on_new_tab>
                "＋"
            </button>
        </div>
    }
}
