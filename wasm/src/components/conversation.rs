use leptos::prelude::*;
use portable::{Message, MessageRole};

use crate::utils::markdown::to_html;

#[component]
pub(crate) fn Conversation(
    messages: RwSignal<Vec<Message>>,
    conv_ref: NodeRef<leptos::html::Div>,
) -> impl IntoView {
    view! {
        <div class="conversation" node_ref=conv_ref>
            {move || messages.get().into_iter()
                .filter(|m| m.role != MessageRole::System)
                .map(|msg| {
                    let row_cls    = format!("msg-row {}",    msg.role);
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
    }
}
