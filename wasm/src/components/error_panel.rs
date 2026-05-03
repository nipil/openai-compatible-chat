use leptos::prelude::*;

/// Error panel: manages its own visibility and closing behavior.
/// The is_visible memo is defined here rather than in App.
#[component]
pub(crate) fn ErrorPanel(errors: RwSignal<Vec<String>>) -> impl IntoView {
    let is_visible = Memo::new(move |_| !errors.with(Vec::is_empty));
    let on_close = move |_| errors.set(vec![]);

    view! {
        <div
            class="error-panel"
            class:error-panel--visible=is_visible
        >

            <div class="error-panel__header">
                <span class="error-panel__title">"Errors"</span>
                <button
                    class="error-panel__close"
                    on:click=on_close
                    aria-label="Dismiss all errors"
                >
                    "\u{2715}"
                </button>
            </div>

            <ul class="error-panel__list">
                {move || errors.get().into_iter()
                    .map(|msg| view! { <li class="error-panel__item">{msg}</li> })
                    .collect::<Vec<_>>()
                }
            </ul>

        </div>
    }
}
