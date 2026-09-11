use leptos::prelude::*;
use serde_json::Value;

use crate::architecture_map::TopologyMapView;
use crate::architecture_model::{module_source_label, non_empty, slot_views};
use crate::types::*;
use crate::ui_utils::compact_json;

#[component]
pub(super) fn TopologySnapshotView(snapshot: TopologySnapshot, source: String) -> impl IntoView {
    let slots = slot_views(&snapshot);
    let slot_cards = slots.clone();

    let model_label = snapshot
        .model
        .as_ref()
        .map(|model| format!("{}/{}", model.provider, model.name))
        .unwrap_or_else(|| "Модель не выбрана".to_owned());
    let registered_tool_count = snapshot.tools.iter().filter(|tool| tool.registered).count();
    let provided_only_count = snapshot.tools.len() - registered_tool_count;
    let process_module_count = snapshot
        .modules
        .iter()
        .filter(|module| module.source.kind == "process")
        .count();

    let tools = snapshot.tools.clone();
    let warnings = snapshot.warnings.clone();
    let (active_tab, set_active_tab) = signal("map");
    let (tool_filter, set_tool_filter) = signal("all".to_owned());
    let (tool_search, set_tool_search) = signal(String::new());
    let tools_for_filter = tools.clone();
    let filtered_tools = move || {
        let filter = tool_filter.get();
        let needle = tool_search.get().trim().to_lowercase();
        tools_for_filter
            .clone()
            .into_iter()
            .filter(|tool| match filter.as_str() {
                "enabled" => tool.enabled && tool.registered,
                "disabled" => !tool.enabled || !tool.registered,
                "read" => tool.safety == "ReadOnly",
                "write" => tool.safety != "ReadOnly",
                "process" => tool.source.contains("process-module"),
                _ => true,
            })
            .filter(|tool| needle.is_empty() || tool.name.to_lowercase().contains(&needle))
            .collect::<Vec<_>>()
    };

    view! {
        <div class="configs-scroll architecture-scroll">
            <section class="architecture-summary" aria-label="Текущая сборка">
                <div><span>"Профиль"</span><strong>{non_empty(&snapshot.profile, "default")}</strong></div>
                <div><span>"Модель"</span><strong>{model_label}</strong></div>
                <div><span>"Состав"</span><strong>{format!("{process_module_count} модулей · {registered_tool_count} инструментов")}</strong></div>
            </section>
            <div class="architecture-tabs" role="group" aria-label="Представление архитектуры">
                <button type="button" aria-pressed=move || (active_tab.get() == "map").to_string() on:click=move |_| set_active_tab.set("map")>"Карта связей"</button>
                <button type="button" aria-pressed=move || (active_tab.get() == "catalog").to_string() on:click=move |_| set_active_tab.set("catalog")>"Каталог сборки"</button>
                <span>{format!("{} предупреждений · {provided_only_count} незарегистрированных инструментов", snapshot.warnings.len())}</span>
            </div>
            <div hidden=move || active_tab.get() != "map"><TopologyMapView source /></div>
            <div class="architecture-catalog" hidden=move || active_tab.get() != "catalog">
            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Слоты сборки"</h3>
                    <span>{slot_cards.len()}</span>
                </div>
                <div class="config-list architecture-card-grid">
                    <For
                        each=move || slot_cards.clone()
                        key=|view| view.slot.id.clone()
                        children=move |view| {
                            let slot_id = view.slot.id.clone();
                            let active_label = view
                                .slot
                                .active_module
                                .clone()
                                .unwrap_or_else(|| "Модуль не выбран".to_owned());
                            let source = view
                                .active_module
                                .as_ref()
                                .map(|module| module_source_label(&module.source))
                                .unwrap_or_else(|| "-".to_owned());
                            let description = view
                                .active_module
                                .as_ref()
                                .and_then(|module| module.description.clone())
                                .unwrap_or_else(|| view.slot.responsibility.clone());
                            let is_active = view.slot.active_module.is_some();
                            let status_class = if is_active {
                                "status-badge completed"
                            } else if view.slot.required {
                                "status-badge failed"
                            } else {
                                "status-badge disconnected"
                            };
                            let status_label = if is_active {
                                "Активен"
                            } else if view.slot.required {
                                "Не выбран"
                            } else {
                                "Отключён"
                            };
                            let required_label = if view.slot.required { "Обязательный" } else { "Необязательный" };
                            let alternatives = view.alternatives.clone();
                            let alternative_count = alternatives.len();
                            view! {
                                <article class="config-list-item topology-tool-item">
                                    <div class="config-list-main">
                                        <div class="config-list-title">
                                            <strong>{slot_id}</strong>
                                            <code>{active_label}</code>
                                            <span class="topology-muted">{source}</span>
                                        </div>
                                        <p>{description}</p>
                                        {if alternatives.is_empty() {
                                            ().into_any()
                                        } else {
                                            view! {
                                                <details class="topology-alternatives">
                                                    <summary>{format!("Другие реализации: {alternative_count}")}</summary>
                                                    <div class="config-chip-row">
                                                        <For
                                                            each=move || alternatives.clone()
                                                            key=|module| format!("{}:{}", module.slot, module.id)
                                                            children=move |module| {
                                                                let label = format!(
                                                                    "{} · {}",
                                                                    module.id,
                                                                    module_source_label(&module.source)
                                                                );
                                                                view! { <span class="config-chip">{label}</span> }
                                                            }
                                                        />
                                                    </div>
                                                </details>
                                            }.into_any()
                                        }}
                                    </div>
                                    <div class="tool-badges">
                                        <span class=status_class>{status_label}</span>
                                        <span class="status-badge idle">{required_label}</span>
                                    </div>
                                </article>
                            }
                        }
                    />
                </div>
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Инструменты"</h3>
                    <span>{tools.len()}</span>
                </div>
                <div class="topology-filter-row">
                    <button type="button" class:active=move || tool_filter.get() == "all" on:click=move |_| set_tool_filter.set("all".to_owned())>"Все"</button>
                    <button type="button" class:active=move || tool_filter.get() == "enabled" on:click=move |_| set_tool_filter.set("enabled".to_owned())>"Включены"</button>
                    <button type="button" class:active=move || tool_filter.get() == "disabled" on:click=move |_| set_tool_filter.set("disabled".to_owned())>"Отключены"</button>
                    <button type="button" class:active=move || tool_filter.get() == "read" on:click=move |_| set_tool_filter.set("read".to_owned())>"Чтение"</button>
                    <button type="button" class:active=move || tool_filter.get() == "write" on:click=move |_| set_tool_filter.set("write".to_owned())>"Изменение"</button>
                    <button type="button" class:active=move || tool_filter.get() == "process" on:click=move |_| set_tool_filter.set("process".to_owned())>"Процессные"</button>
                    <input
                        type="search"
                        class="topology-filter-search"
                        placeholder="Найти инструмент"
                        prop:value=move || tool_search.get()
                        on:input:target=move |ev| set_tool_search.set(ev.target().value())
                    />
                </div>
                <div class="config-list architecture-card-grid">
                    <For
                        each=filtered_tools
                        key=|tool| format!("{}:{}", tool.name, tool.source)
                        children=move |tool| {
                            let registration_class = if tool.registered {
                                "status-badge completed"
                            } else {
                                "status-badge disconnected"
                            };
                            let registration_label = if tool.registered { "Зарегистрирован" } else { "Предоставлен" };
                            let enabled_class = if tool.enabled {
                                "status-badge completed"
                            } else {
                                "status-badge failed"
                            };
                            let enabled_label = if tool.enabled { "Включён" } else { "Отключён" };
                            let source = tool.source.clone();
                            view! {
                                <article class="config-list-item topology-tool-item">
                                    <div class="config-list-main">
                                        <div class="config-list-title">
                                            <strong>{tool.name.clone()}</strong>
                                            <code>{source}</code>
                                        </div>
                                        <p>{tool.description.clone()}</p>
                                        <details class="topology-schema">
                                            <summary>"Схема параметров"</summary>
                                            <pre>{schema_json(&tool.input_schema)}</pre>
                                        </details>
                                    </div>
                                    <div class="tool-badges">
                                        <span class="status-badge idle">{tool.safety}</span>
                                        <span class=registration_class>{registration_label}</span>
                                        <span class=enabled_class>{enabled_label}</span>
                                    </div>
                                </article>
                            }
                        }
                    />
                </div>
            </section>

            </div>
            {(!warnings.is_empty())
                .then(|| {
                    let warnings = warnings.clone();
                    let warning_count = warnings.len();
                    view! {
                        <section class="config-section">
                            <div class="config-section-header">
                                <h3>"Предупреждения"</h3>
                                <span>{warning_count}</span>
                            </div>
                            <div class="config-list">
                                <For
                                    each=move || warnings.clone()
                                    key=|warning| format!("{}:{}", warning.severity, warning.message)
                                    children=move |warning| {
                                        let badge_class = if warning.severity == "error" {
                                            "status-badge failed"
                                        } else {
                                            "status-badge disconnected"
                                        };
                                        view! {
                                            <article class="config-list-item">
                                                <div class="config-list-main">
                                                    <div class="config-list-title">
                                                        <strong>{warning.severity.clone()}</strong>
                                                    </div>
                                                    <p>{warning.message}</p>
                                                </div>
                                                <span class=badge_class>{warning.severity}</span>
                                            </article>
                                        }
                                    }
                                />
                            </div>
                        </section>
                    }
                })}


        </div>
    }
}

fn schema_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| compact_json(value))
}
