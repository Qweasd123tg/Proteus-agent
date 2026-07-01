use std::collections::{BTreeMap, BTreeSet};

use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;

use crate::api::{get_json, post_json};
use crate::types::*;

use super::module_config_editor::ModuleConfigEditor;

#[component]
pub(super) fn ConfigBuilderView(
    builder: ConfigBuilderSnapshot,
    draft_modules: ReadSignal<BTreeMap<String, String>>,
    set_draft_modules: WriteSignal<BTreeMap<String, String>>,
    draft_config_texts: ReadSignal<BTreeMap<String, String>>,
    set_draft_config_texts: WriteSignal<BTreeMap<String, String>>,
    draft_module_config: ReadSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    set_draft_module_config: WriteSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    draft_tools: ReadSignal<BTreeSet<String>>,
    set_draft_tools: WriteSignal<BTreeSet<String>>,
    set_builder: WriteSignal<Option<ConfigBuilderSnapshot>>,
    set_summary: WriteSignal<Option<ConfigSummary>>,
    set_status: WriteSignal<String>,
) -> impl IntoView {
    let slots = builder.slots.clone();
    let tools = builder.tools.clone();
    let warnings = builder.warnings.clone();
    let target_path = builder
        .target_path
        .clone()
        .unwrap_or_else(|| "(config path unavailable)".to_owned());
    let writable = builder.writable;

    let save = move |_| {
        if !writable {
            set_status.set("config path недоступен для записи".to_owned());
            return;
        }
        let modules = draft_modules.get_untracked();
        let mut module_config = draft_module_config.get_untracked();
        let text_by_slot = draft_config_texts.get_untracked();
        let tools_enabled = draft_tools.get_untracked().into_iter().collect::<Vec<_>>();
        let mut errors = Vec::new();

        for (slot, module_id) in &modules {
            let text = text_by_slot
                .get(slot)
                .map(String::as_str)
                .unwrap_or("{}")
                .trim();
            let value = if text.is_empty() {
                Value::Object(Default::default())
            } else {
                match serde_json::from_str::<Value>(text) {
                    Ok(value) => value,
                    Err(error) => {
                        errors.push(format!("{slot}/{module_id}: {error}"));
                        continue;
                    }
                }
            };
            module_config
                .entry(slot.clone())
                .or_default()
                .insert(module_id.clone(), value);
        }

        if !errors.is_empty() {
            set_status.set(format!("JSON error: {}", errors.join("; ")));
            return;
        }

        set_status.set("сохраняю config builder".to_owned());
        spawn_local(async move {
            let request = ConfigBuilderSaveRequest {
                modules,
                module_config,
                tools_enabled: Some(tools_enabled),
            };
            match post_json::<_, ConfigBuilderSnapshot>("/config/builder", &request).await {
                Ok(next_builder) => {
                    let next_modules = builder_active_modules(&next_builder);
                    let next_texts = builder_config_texts(&next_builder, &next_modules);
                    set_draft_module_config.set(next_builder.module_config.clone());
                    set_draft_modules.set(next_modules);
                    set_draft_config_texts.set(next_texts);
                    set_draft_tools.set(next_builder.tools_enabled.iter().cloned().collect());
                    set_builder.set(Some(next_builder));
                    match get_json::<ConfigSummary>("/config").await {
                        Ok(summary) => set_summary.set(Some(summary)),
                        Err(error) => {
                            set_status.set(format!("сохранено, summary не обновился: {error}"));
                            return;
                        }
                    }
                    set_status.set("config builder сохранён и runtime перезагружен".to_owned());
                }
                Err(error) => set_status.set(format!("не удалось сохранить builder: {error}")),
            }
        });
    };

    view! {
        <section class="config-section config-builder">
            <div class="config-section-header">
                <h3>"Config builder"</h3>
                <span>{if writable { "writable" } else { "readonly" }}</span>
            </div>
            <div class="config-builder-target">
                <span>"target"</span>
                <code>{target_path}</code>
                <button type="button" class="btn-primary" disabled=!writable on:click=save>"Сохранить"</button>
            </div>
            {if warnings.is_empty() {
                view! { <div></div> }.into_any()
            } else {
                view! {
                    <div class="config-builder-warnings">
                        <For
                            each=move || warnings.clone()
                            key=|warning| format!("{}:{}", warning.severity, warning.message)
                            children=move |warning| {
                                view! {
                                    <div class="config-builder-warning">
                                        <span>{warning.severity}</span>
                                        <p>{warning.message}</p>
                                    </div>
                                }
                            }
                        />
                    </div>
                }.into_any()
            }}
            <div class="config-builder-grid">
                <For
                    each=move || slots.clone()
                    key=|slot| slot.id.clone()
                    children=move |slot| {
                        view! {
                            <BuilderSlotCard
                                builder_slot=slot
                                draft_modules
                                set_draft_modules
                                draft_config_texts
                                set_draft_config_texts
                                draft_module_config
                            />
                        }
                    }
                />
            </div>
            <ToolsPicker tools draft_tools set_draft_tools/>
        </section>
    }
}

#[component]
fn BuilderSlotCard(
    builder_slot: ConfigBuilderSlot,
    draft_modules: ReadSignal<BTreeMap<String, String>>,
    set_draft_modules: WriteSignal<BTreeMap<String, String>>,
    draft_config_texts: ReadSignal<BTreeMap<String, String>>,
    set_draft_config_texts: WriteSignal<BTreeMap<String, String>>,
    draft_module_config: ReadSignal<BTreeMap<String, BTreeMap<String, Value>>>,
) -> impl IntoView {
    let slot = builder_slot;
    let slot_id = slot.id.clone();
    let slot_id_for_select_value = slot_id.clone();
    let slot_id_for_select_change = slot_id.clone();
    let slot_id_for_label = slot_id.clone();
    let modules_for_select = slot.modules.clone();
    let modules_for_capabilities = slot.modules.clone();
    let module_count = slot.modules.len();
    let capabilities_slot_id = slot_id.clone();

    view! {
        <article class="config-builder-slot">
            <div class="config-builder-slot-head">
                <div>
                    <span class="panel-kicker">{slot.category.clone()}</span>
                    <strong>{slot.title.clone()}</strong>
                </div>
                <code>{slot.id.clone()}</code>
            </div>
            <p>{slot.responsibility.clone()}</p>
            <label class="config-builder-field">
                <span>{format!("{} module", slot_id_for_label)}</span>
                <select
                    prop:value=move || {
                        draft_modules
                            .with(|items| items.get(&slot_id_for_select_value).cloned())
                            .unwrap_or_default()
                    }
                    on:change:target=move |ev| {
                        let selected = ev.target().value();
                        set_draft_modules.update(|items| {
                            items.insert(slot_id_for_select_change.clone(), selected.clone());
                        });
                        let text = draft_module_config.with(|config| {
                            module_config_text(config, &slot_id_for_select_change, &selected)
                        });
                        set_draft_config_texts.update(|items| {
                            items.insert(slot_id_for_select_change.clone(), text);
                        });
                    }
                >
                    <For
                        each=move || modules_for_select.clone()
                        key=|module| module.id.clone()
                        children=move |module| {
                            view! {
                                <option value=module.id.clone()>{module_option_label(&module)}</option>
                            }
                        }
                    />
                </select>
            </label>
            <ModuleConfigEditor slot_id=slot_id.clone() draft_config_texts set_draft_config_texts/>
            <div class="config-builder-modules">
                <span>{format!("{module_count} candidates")}</span>
                <div class="config-chip-row">
                    <For
                        each=move || {
                            let active = draft_modules
                                .with(|items| items.get(&capabilities_slot_id).cloned())
                                .unwrap_or_default();
                            modules_for_capabilities
                                .iter()
                                .find(|module| module.id == active)
                                .map(|module| module.capabilities.clone())
                                .unwrap_or_default()
                        }
                        key=|capability| capability.clone()
                        children=move |capability| view! { <span class="config-chip">{capability}</span> }
                    />
                </div>
            </div>
        </article>
    }
}

/// Каталог tools с чекбоксами `tools.enabled`. Показывает и tools, которые
/// включены в config, но не registered в runtime (например, plugin выключен).
#[component]
fn ToolsPicker(
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

pub(super) fn builder_active_modules(builder: &ConfigBuilderSnapshot) -> BTreeMap<String, String> {
    builder
        .active_modules
        .iter()
        .map(|module| (module.slot.clone(), module.id.clone()))
        .collect()
}

pub(super) fn builder_config_texts(
    builder: &ConfigBuilderSnapshot,
    modules: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    modules
        .iter()
        .map(|(slot, module_id)| {
            (
                slot.clone(),
                module_config_text(&builder.module_config, slot, module_id),
            )
        })
        .collect()
}

fn module_config_text(
    config: &BTreeMap<String, BTreeMap<String, Value>>,
    slot: &str,
    module_id: &str,
) -> String {
    config
        .get(slot)
        .and_then(|slot_config| slot_config.get(module_id))
        .map(pretty_json)
        .unwrap_or_else(|| "{\n}".to_owned())
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned())
}

fn module_option_label(module: &ConfigBuilderModule) -> String {
    if module.source.trim().is_empty() {
        module.id.clone()
    } else {
        format!("{} · {}", module.id, module.source)
    }
}
