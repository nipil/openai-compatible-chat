use leptos::prelude::*;

#[component]
pub fn SystemPrompt(sys_prompt: RwSignal<String>, started: RwSignal<bool>) -> impl IntoView {
    view! {
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
    }
}
