use leptos::prelude::*;
use web_sys::KeyboardEvent;

use crate::utils::keyboard::KeyboardId;
use crate::{AppError, handle_err_clos_1};

/// InputArea receives on_send, which returns a Result to be wrapped
/// by handle_err_clos_1 on keydown (Ctrl+Enter) or click on Send.
#[component]
pub fn InputArea(
    errors: RwSignal<Vec<String>>,
    input: RwSignal<String>,
    streaming: RwSignal<bool>,
    on_send: impl Fn() -> Result<(), AppError> + 'static + Copy + Send,
    on_stop: impl Fn() + 'static + Copy + Send,
) -> impl IntoView {
    view! {
        <div class="input-area">
            <textarea
                class="input-textarea"
                placeholder="Message… (Ctrl+Enter to send  •  Esc to stop)"
                prop:value=move || input.get()
                on:input=move |e| input.set(event_target_value(&e))
                on:keydown=handle_err_clos_1(errors, move |e: KeyboardEvent| {
                    if e.ctrl_key()
                        && e.key() == KeyboardId::Enter.as_ref()
                        && !streaming.get_untracked()
                    {
                        on_send()?;
                    }
                    Ok(())
                })
                rows="3"
            />
            {move || if streaming.get() {
                // FIXME
                view! {
                    <button class="btn-stop" on:click=move |_| on_stop()>
                        "⏹ Stop (Esc)"
                    </button>
                }.into_any()
            } else {
                // FIXME
                view! {
                    <button
                        class="btn-send"
                        on:click=handle_err_clos_1(errors, move |_| on_send())
                    >
                        "Send ↵"
                    </button>
                }.into_any()
            }}
        </div>
    }
}
