use std::collections::BTreeSet;

use leptos::prelude::*;

use crate::types::*;

/// Каталог tools с чекбоксами `tools.enabled`. Показывает и tools, которые
/// включены в config, но не registered в runtime (например, plugin выключен).
#[component]
pub(super) fn ToolsPicker(
    tools: Vec<ConfigBuilderTool>,
    draft_tools: ReadSignal<BTreeSet<String>>,
    set_draft_tools: WriteSignal<BTreeSet<String>>,
) -> impl IntoView {
    let (filter, set_filter) = signal(String::new());
    let total = tools.len();
    let known = tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<BTreeSet<_>>();

    let rows = move || {
        let mut rows = tools.clone();
        draft_tools.with(|draft| {
            for name in draft {
                if !known.contains(name) {
                    rows.push(ConfigBuilderTool {
                        name: name.clone(),
                        source: "config".to_owned(),
                        safety: "-".to_owned(),
                        description: "включён в config, но не registered в runtime".to_owned(),
                        enabled: true,
                        registered: false,
                    });
                }
            }
        });
        let needle = filter.get().trim().to_lowercase();
        if !needle.is_empty() {
            rows.retain(|tool| tool.name.to_lowercase().contains(&needle));
        }
        rows
    };

    view! {
        <div class="tools-picker">
            <div class="tools-picker-head">
                <div>
                    <strong>"Tools"</strong>
                    <span>
                        {move || draft_tools.with(BTreeSet::len)}
                        " включено · "
                        {total}
                        " в каталоге"
                    </span>
                </div>
                <input
                    type="search"
                    placeholder="фильтр по имени"
                    prop:value=move || filter.get()
                    on:input:target=move |ev| set_filter.set(ev.target().value())
                />
            </div>
            <div class="tools-picker-list">
                <For
                    each=rows
                    key=|tool| tool.name.clone()
                    children=move |tool| {
                        let name_for_checked = tool.name.clone();
                        let name_for_toggle = tool.name.clone();
                        view! {
                            <label class="tools-picker-row">
                                <input
                                    type="checkbox"
                                    prop:checked=move || {
                                        draft_tools.with(|draft| draft.contains(&name_for_checked))
                                    }
                                    on:change:target=move |ev| {
                                        let checked = ev.target().checked();
                                        let name = name_for_toggle.clone();
                                        set_draft_tools.update(|draft| {
                                            if checked {
                                                draft.insert(name);
                                            } else {
                                                draft.remove(&name);
                                            }
                                        });
                                    }
                                />
                                <div class="tools-picker-main">
                                    <div class="tools-picker-title">
                                        <strong>{tool.name.clone()}</strong>
                                        <code>{tool.source.clone()}</code>
                                        <span class="status-badge idle">{tool.safety.clone()}</span>
                                        {(!tool.registered)
                                            .then(|| {
                                                view! {
                                                    <span class="status-badge failed">"не registered"</span>
                                                }
                                            })}
                                    </div>
                                    <p>{tool.description.clone()}</p>
                                </div>
                            </label>
                        }
                    }
                />
            </div>
        </div>
    }
}
