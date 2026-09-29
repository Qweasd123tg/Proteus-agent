//! Only viewport rows own message rendering and reactive subscriptions.
use super::{
    MessageView,
    tool_chain::{Group, ToolChain},
    transcript_state::TranscriptRowId,
};
use crate::transcript::Transcript;
use leptos::{html, prelude::*};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Serialize)]
struct Row {
    id: String,
    height: u32,
    owner: Option<u64>,
}

#[derive(Clone, PartialEq, Deserialize)]
struct VisibleRow {
    index: usize,
    gap: f64,
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
    let range = RwSignal::new(None::<Vec<VisibleRow>>);
    let rows = Memo::new(move |_| {
        groups.with(|groups| {
            let mut owner = None;
            groups
                .iter()
                .map(|group| {
                    if group.role == crate::types::MessageRole::User {
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
    });
    let owners = Memo::new(move |_| {
        rows.with(|rows| {
            rows.iter()
                .map(|row| (row.id.clone(), row.owner))
                .collect::<std::collections::HashMap<_, _>>()
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
                let visible = range.get();
                groups.with(|groups| {
                    let visible = visible.unwrap_or_else(|| (groups.len().saturating_sub(24)..groups.len()).map(|index| VisibleRow { index, gap: 0.0 }).collect());
                    visible.into_iter().filter_map(|row| groups.get(row.index).cloned().map(|group| (session.clone(), group))).collect::<Vec<_>>()
                })
            }
            key=|(session, group)| (session.clone(), group.id, group.tools)
            children=move |(_, group)| {
                let id = group.id;
                provide_context(TranscriptRowId(id));
                let owner = move || owners.with(|owners| owners.get(&id.to_string()).copied().flatten().map(|id| format!("msg-{id}")));
                let gap = move || range.with(|visible| visible.as_ref().and_then(|visible| groups.with(|groups| visible.iter().find(|row| groups.get(row.index).is_some_and(|group| group.id == id)).map(|row| row.gap))).unwrap_or_default());
                view! {
                    <div class="transcript-spacer" data-transcript-gap=id.to_string() aria-hidden="true" style:height=move ||format!("{}px",gap())></div>
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
    range: RwSignal<Option<Vec<VisibleRow>>>,
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
        let on_range = Closure::wrap(Box::new(move |visible: String| {
            range.set(Some(
                serde_json::from_str(&visible).expect("visible transcript rows"),
            ));
        }) as Box<dyn FnMut(String)>);
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
