use std::collections::BTreeSet;

use leptos::prelude::*;

use crate::types::*;

#[component]
pub(super) fn ToolsPicker(
    tools: Vec<ConfigBuilderTool>,
    draft_tools: ReadSignal<BTreeSet<String>>,
    set_draft_tools: WriteSignal<BTreeSet<String>>,
) -> impl IntoView {
    let search = RwSignal::new(String::new());
    let enabled_only = RwSignal::new(false);
    let total = tools.len();
    let known = tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<BTreeSet<_>>();
    let rows = Memo::new(move |_| {
        tool_rows(
            &tools,
            &known,
            &draft_tools.get(),
            &search.get(),
            enabled_only.get(),
        )
    });

    view! {
        <div class="tools-picker">
            <div class="tools-picker-head">
                <div>
                    <strong>"Инструменты"</strong>
                    <span>{move || format!("{} включено · {} доступно", draft_tools.with(BTreeSet::len), total)}</span>
                </div>
                <input type="search" placeholder="Поиск по имени или описанию" aria-label="Поиск инструментов"
                    prop:value=move || search.get() on:input:target=move |ev| search.set(ev.target().value())/>
            </div>
            <div class="cfg-filter-buttons" role="group" aria-label="Фильтр инструментов">
                <button type="button" class="cfg-filter-button" class:active=move || !enabled_only.get()
                    aria-pressed=move || (!enabled_only.get()).to_string() on:click=move |_| enabled_only.set(false)>
                    "Все" <span>{total}</span>
                </button>
                <button type="button" class="cfg-filter-button" class:active=move || enabled_only.get()
                    aria-pressed=move || enabled_only.get().to_string() on:click=move |_| enabled_only.set(true)>
                    "Включённые" <span>{move || draft_tools.with(BTreeSet::len)}</span>
                </button>
            </div>
            <div class="tools-picker-list">
                <For each=move || rows.get() key=|tool| tool.name.clone() children=move |tool| {
                    let checked_name = tool.name.clone();
                    let toggle_name = tool.name.clone();
                    view! {
                        <label class="tools-picker-row" class:unavailable=!tool.registered>
                            <input type="checkbox" prop:checked=move || draft_tools.with(|draft| draft.contains(&checked_name))
                                on:change:target=move |ev| {
                                    let checked = ev.target().checked(); let name = toggle_name.clone();
                                    set_draft_tools.update(|draft| { if checked { draft.insert(name); } else { draft.remove(&name); } });
                                }/>
                            <div class="tools-picker-main">
                                <div class="tools-picker-title">
                                    <strong>{tool.name.clone()}</strong>
                                    <code>{tool.source.clone()}</code>
                                    <span class="status-badge idle">{tool.safety.clone()}</span>
                                    {(!tool.registered).then(|| view! { <span class="status-badge failed">"Недоступен в runtime"</span> })}
                                </div>
                                <p>{tool.description.clone()}</p>
                            </div>
                        </label>
                    }
                }/>
                <Show when=move || rows.get().is_empty()>
                    <div class="config-empty">"Инструменты по этому фильтру не найдены"</div>
                </Show>
            </div>
        </div>
    }
}

fn tool_rows(
    tools: &[ConfigBuilderTool],
    known: &BTreeSet<String>,
    enabled: &BTreeSet<String>,
    query: &str,
    enabled_only: bool,
) -> Vec<ConfigBuilderTool> {
    let mut rows = tools.to_vec();
    for name in enabled {
        if !known.contains(name) {
            rows.push(ConfigBuilderTool {
                name: name.clone(),
                source: "config".to_owned(),
                safety: "-".to_owned(),
                description: "Включён в конфигурации, но сейчас недоступен в runtime".to_owned(),
                enabled: true,
                registered: false,
            });
        }
    }
    let needle = query.trim().to_lowercase();
    rows.retain(|tool| {
        (!enabled_only || enabled.contains(&tool.name))
            && (needle.is_empty()
                || tool.name.to_lowercase().contains(&needle)
                || tool.description.to_lowercase().contains(&needle))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_description_and_retains_unavailable_enabled_tools() {
        let tools = vec![ConfigBuilderTool {
            name: "read_file".into(),
            description: "Чтение файлов".into(),
            registered: true,
            ..Default::default()
        }];
        let known = BTreeSet::from(["read_file".into()]);
        let enabled = BTreeSet::from(["missing_tool".into()]);
        assert_eq!(
            tool_rows(&tools, &known, &enabled, "файл", false)[0].name,
            "read_file"
        );
        let rows = tool_rows(&tools, &known, &enabled, "", true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "missing_tool");
        assert!(!rows[0].registered);
    }
}
