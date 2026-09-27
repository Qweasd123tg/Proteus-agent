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
    let count_tools = tools.clone();
    let count_known = known.clone();
    let enabled_count =
        Memo::new(move |_| tool_enabled_count(&count_tools, &count_known, &draft_tools.get()));
    let all_known = known.clone();
    let all_count = Memo::new(move |_| {
        total
            + draft_tools.with(|draft| {
                draft
                    .iter()
                    .filter(|name| !all_known.contains(*name))
                    .count()
            })
    });
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
                    <span>{move || format!("{} включено · {} доступно", enabled_count.get(), total)}</span>
                </div>
                <input type="search" placeholder="Поиск по имени или описанию" aria-label="Поиск инструментов"
                    prop:value=move || search.get() on:input:target=move |ev| search.set(ev.target().value())/>
            </div>
            <div class="cfg-filter-buttons" role="group" aria-label="Фильтр инструментов">
                <button type="button" class="cfg-filter-button" class:active=move || !enabled_only.get()
                    aria-pressed=move || (!enabled_only.get()).to_string() on:click=move |_| enabled_only.set(false)>
                    "Все" <span>{move || all_count.get()}</span>
                </button>
                <button type="button" class="cfg-filter-button" class:active=move || enabled_only.get()
                    aria-pressed=move || enabled_only.get().to_string() on:click=move |_| enabled_only.set(true)>
                    "Включённые" <span>{move || enabled_count.get()}</span>
                </button>
            </div>
            <div class="tools-picker-list">
                <For each=move || rows.get() key=|tool| tool.name.clone() children=move |tool| {
                    let checked_name = tool.name.clone();
                    let toggle_name = tool.name.clone();
                    let runtime_managed = tool.runtime_managed;
                    view! {
                        <label class="tools-picker-row" class:unavailable=!tool.registered>
                            <input type="checkbox" disabled=runtime_managed
                                title=runtime_managed.then_some("Управляется runtime; tools.enabled не отключает инструмент")
                                prop:checked=move || draft_tools.with(|draft| runtime_managed || draft.contains(&checked_name))
                                on:change:target=move |ev| {
                                    if runtime_managed { return; }
                                    let checked = ev.target().checked(); let name = toggle_name.clone();
                                    set_draft_tools.update(|draft| { if checked { draft.insert(name); } else { draft.remove(&name); } });
                                }/>
                            <div class="tools-picker-main">
                                <div class="tools-picker-title">
                                    <strong>{tool.name.clone()}</strong>
                                    <code>{tool.source.clone()}</code>
                                    <span class="status-badge idle">{tool.safety.clone()}</span>
                                    {runtime_managed.then(|| view! { <span class="status-badge idle" title="Управляется runtime; tools.enabled не отключает инструмент">"Управляется runtime"</span> })}
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
                runtime_managed: false,
                registered: false,
            });
        }
    }
    let needle = query.trim().to_lowercase();
    rows.retain(|tool| {
        (!enabled_only || tool_is_enabled(tool, enabled))
            && (needle.is_empty()
                || tool.name.to_lowercase().contains(&needle)
                || tool.description.to_lowercase().contains(&needle))
    });
    rows
}

fn tool_is_enabled(tool: &ConfigBuilderTool, draft: &BTreeSet<String>) -> bool {
    tool.runtime_managed || draft.contains(&tool.name)
}

fn tool_enabled_count(
    tools: &[ConfigBuilderTool],
    known: &BTreeSet<String>,
    draft: &BTreeSet<String>,
) -> usize {
    tools
        .iter()
        .filter(|tool| tool_is_enabled(tool, draft))
        .count()
        + draft.iter().filter(|name| !known.contains(*name)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_and_configured_tools_have_stable_enabled_state() {
        let tools = vec![
            ConfigBuilderTool {
                name: "web_search".into(),
                runtime_managed: true,
                registered: true,
                ..Default::default()
            },
            ConfigBuilderTool {
                name: "mcp_tool".into(),
                runtime_managed: true,
                registered: true,
                ..Default::default()
            },
            ConfigBuilderTool {
                name: "read_file".into(),
                description: "Чтение файлов".into(),
                registered: true,
                ..Default::default()
            },
        ];
        let known = tools.iter().map(|tool| tool.name.clone()).collect();
        // One managed name is persisted already. The other must never be added by this picker.
        let mut draft = BTreeSet::from([
            "web_search".into(),
            "read_file".into(),
            "missing_tool".into(),
        ]);
        assert_eq!(tool_enabled_count(&tools, &known, &draft), 4);
        assert_eq!(tool_rows(&tools, &known, &draft, "", true).len(), 4);
        assert_eq!(
            tool_rows(&tools, &known, &draft, "файл", false)[0].name,
            "read_file"
        );
        assert_eq!(
            tool_rows(&tools, &known, &draft, "mcp", true)[0].name,
            "mcp_tool"
        );
        assert!(!tool_rows(&tools, &known, &draft, "missing", true)[0].registered);

        draft.remove("read_file");
        draft.remove("web_search");
        assert_eq!(tool_enabled_count(&tools, &known, &draft), 3);
        assert!(!tool_is_enabled(&tools[2], &draft));
        assert!(!tools[2].runtime_managed);
        assert!(tool_is_enabled(&tools[0], &draft));
        assert!(tool_is_enabled(&tools[1], &draft));
        assert!(!draft.contains("mcp_tool"));
        assert!(tool_rows(&tools, &known, &draft, "read", true).is_empty());
    }
}
