//! Only viewport rows own message rendering and reactive subscriptions.
use super::{
    MessageView,
    tool_chain::{Group, ToolChain},
    transcript_state::TranscriptRowId,
};
use crate::transcript::Transcript;
use leptos::{html, prelude::*};
use serde::Serialize;

#[derive(Clone, PartialEq, Serialize)]
struct Row {
    id: String,
    height: u32,
    owner: Option<u64>,
}

#[component]
pub(super) fn VirtualTranscript(
    root: NodeRef<html::Section>,
    groups: Memo<Vec<Group>>,
    messages: Transcript,
    activity_now_ms: ReadSignal<u64>,
    session: ReadSignal<Option<String>>,
    set_last_scroll_top: WriteSignal<i32>,
) -> impl IntoView {
    let range = RwSignal::new((usize::MAX, usize::MAX));
    let rows = Memo::new(move |_| {
        groups.with(|groups| {
            let mut owner = None;
            messages.with_untracked(|items| {
                let users = items
                    .iter()
                    .filter(|item| item.role == crate::types::MessageRole::User)
                    .map(|item| item.id)
                    .collect::<std::collections::HashSet<_>>();
                groups
                    .iter()
                    .map(|group| {
                        if users.contains(&group.id) {
                            owner = Some(group.id);
                        }
                        Row {
                            id: group.id.to_string(),
                            height: if group.tools { 48 } else { 144 },
                            owner,
                        }
                    })
                    .collect::<Vec<_>>()
            })
        })
    });
    #[cfg(target_arch = "wasm32")]
    attach(root, rows, range, session, set_last_scroll_top);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (root, rows, set_last_scroll_top);
    view! {
        <div class="transcript-spacer" data-transcript-top="" aria-hidden="true"></div>
        <For
            each=move || {
                let session = session.get();
                let (start, end) = range.get();
                groups.with(|groups| {
                    let start = if start == usize::MAX { groups.len().saturating_sub(24) } else { start.min(groups.len()) };
                    let end = end.min(groups.len()).max(start);
                    groups[start..end].iter().cloned().map(|group| (session.clone(), group)).collect::<Vec<_>>()
                })
            }
            key=|(session, group)| (session.clone(), group.id, group.tools)
            children=move |(_, group)| {
                let id = group.id;
                provide_context(TranscriptRowId(id));
                let owner = move || rows.with(|rows| rows.iter().find(|row| row.id == id.to_string()).and_then(|row| row.owner).map(|id| format!("msg-{id}")));
                view! {
                    <div class="transcript-row" data-transcript-row=id.to_string() data-prompt-id=owner>
                        {if group.tools {
                            view! { <ToolChain id groups messages activity_now_ms/> }.into_any()
                        } else {
                            view! { <MessageView message_id=id messages activity_now_ms/> }.into_any()
                        }}
                    </div>
                }
            }
        />
        <div class="transcript-spacer" data-transcript-bottom="" aria-hidden="true"></div>
    }
}

#[cfg(target_arch = "wasm32")]
fn attach(
    root: NodeRef<html::Section>,
    rows: Memo<Vec<Row>>,
    range: RwSignal<(usize, usize)>,
    session: ReadSignal<Option<String>>,
    last: WriteSignal<i32>,
) {
    use wasm_bindgen::{JsCast, closure::Closure, prelude::*};
    #[wasm_bindgen(raw_module = "/ui/virtual-transcript.js")]
    extern "C" {
        #[wasm_bindgen(js_name = mountVirtualTranscript)]
        fn mount(
            root: &web_sys::Element,
            range: &js_sys::Function,
            adjusted: &js_sys::Function,
        ) -> js_sys::Function;
        #[wasm_bindgen(js_name = updateVirtualTranscript)]
        fn update(root: &web_sys::Element, rows: &str, session: &str);
    }
    Effect::new(move |_| {
        let Some(element) = root.get() else { return };
        let on_range =
            Closure::wrap(
                Box::new(move |start: usize, end: usize| range.set((start, end)))
                    as Box<dyn FnMut(usize, usize)>,
            );
        let on_adjusted =
            Closure::wrap(Box::new(move |top: i32| last.set(top)) as Box<dyn FnMut(i32)>);
        let dispose = mount(
            element.as_ref(),
            on_range.as_ref().unchecked_ref(),
            on_adjusted.as_ref().unchecked_ref(),
        );
        let lifetime = StoredValue::new_local((dispose, on_range, on_adjusted));
        on_cleanup(move || {
            lifetime.with_value(|(dispose, _, _)| {
                let _ = dispose.call0(&JsValue::NULL);
            });
        });
        Effect::new(move |_| {
            let session = session.get().unwrap_or_default();
            if let Some(element) = root.get_untracked() {
                rows.with(|rows| {
                    update(
                        element.as_ref(),
                        &serde_json::to_string(rows).expect("transcript rows"),
                        &session,
                    )
                });
            }
        });
    });
}
