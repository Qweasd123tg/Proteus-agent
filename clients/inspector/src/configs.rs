use std::collections::BTreeMap;

use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;

use crate::api::{get_json, post_json};
use crate::types::*;
use crate::ui_utils::short_path;

#[component]
pub(crate) fn ConfigsView() -> impl IntoView {
    let (summary, set_summary) = signal(None::<ConfigSummary>);
    let (builder, set_builder) = signal(None::<ConfigBuilderSnapshot>);
    let (draft_modules, set_draft_modules) = signal(BTreeMap::<String, String>::new());
    let (draft_config_texts, set_draft_config_texts) =
        signal(BTreeMap::<String, String>::new());
    let (draft_module_config, set_draft_module_config) =
        signal(BTreeMap::<String, BTreeMap<String, Value>>::new());
    let (status, set_status) = signal("загружаю конфигурацию".to_owned());

    load_config_page(
        set_summary,
        set_builder,
        set_draft_modules,
        set_draft_config_texts,
        set_draft_module_config,
        set_status,
    );

    let refresh = move |_| {
        load_config_page(
            set_summary,
            set_builder,
            set_draft_modules,
            set_draft_config_texts,
            set_draft_module_config,
            set_status,
        )
    };

    view! {
        <section class="configs-page">
            <div class="resume-toolbar">
                <div>
                    <h2>"Configs"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <button type="button" class="secondary" on:click=refresh>"Обновить"</button>
            </div>
            {move || {
                summary
                    .get()
                    .map(|summary| {
                        view! {
                            <ConfigSummaryView
                                summary
                                builder=builder.get()
                                draft_modules
                                set_draft_modules
                                draft_config_texts
                                set_draft_config_texts
                                draft_module_config
                                set_draft_module_config
                                set_builder
                                set_summary
                                set_status
                            />
                        }
                        .into_any()
                    })
                    .unwrap_or_else(|| {
                        view! {
                            <div class="empty-state">
                                <div class="empty-state-title">"Config summary недоступен"</div>
                            </div>
                        }
                        .into_any()
                    })
            }}
        </section>
    }
}

fn load_config_page(
    set_summary: WriteSignal<Option<ConfigSummary>>,
    set_builder: WriteSignal<Option<ConfigBuilderSnapshot>>,
    set_draft_modules: WriteSignal<BTreeMap<String, String>>,
    set_draft_config_texts: WriteSignal<BTreeMap<String, String>>,
    set_draft_module_config: WriteSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    set_status: WriteSignal<String>,
) {
    spawn_local(async move {
        match get_json::<ConfigSummary>("/config").await {
            Ok(summary) => {
                let module_count = summary.modules.len();
                let tool_count = summary.registered_tools.len();
                let plugin_count = summary.plugins.len();
                set_summary.set(Some(summary));
                set_status.set(format!(
                    "{module_count} modules · {tool_count} tools · {plugin_count} plugins"
                ));
            }
            Err(error) => {
                set_summary.set(None);
                set_builder.set(None);
                set_status.set(format!("не удалось загрузить config: {error}"));
                return;
            }
        }

        match get_json::<ConfigBuilderSnapshot>("/config/builder").await {
            Ok(builder) => {
                let modules = builder_active_modules(&builder);
                let texts = builder_config_texts(&builder, &modules);
                set_draft_module_config.set(builder.module_config.clone());
                set_draft_modules.set(modules);
                set_draft_config_texts.set(texts);
                let slot_count = builder.slots.len();
                let target = builder
                    .target_path
                    .as_deref()
                    .map(short_path)
                    .unwrap_or_else(|| "без файла".to_owned());
                set_builder.set(Some(builder));
                set_status.set(format!("{slot_count} slots · target {target}"));
            }
            Err(error) => {
                set_builder.set(None);
                set_status.set(format!("summary загружен, builder недоступен: {error}"));
            }
        }
    });
}

#[component]
fn ConfigSummaryView(
    summary: ConfigSummary,
    builder: Option<ConfigBuilderSnapshot>,
    draft_modules: ReadSignal<BTreeMap<String, String>>,
    set_draft_modules: WriteSignal<BTreeMap<String, String>>,
    draft_config_texts: ReadSignal<BTreeMap<String, String>>,
    set_draft_config_texts: WriteSignal<BTreeMap<String, String>>,
    draft_module_config: ReadSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    set_draft_module_config: WriteSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    set_builder: WriteSignal<Option<ConfigBuilderSnapshot>>,
    set_summary: WriteSignal<Option<ConfigSummary>>,
    set_status: WriteSignal<String>,
) -> impl IntoView {
    let model_label = non_empty(summary.model.label.as_str(), "model не выбран");
    let config_path = summary
        .config_path
        .as_deref()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "(default discovery / none)".to_owned());
    let reasoning_effort = summary
        .reasoning
        .effort
        .as_deref()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "auto".to_owned());
    let reasoning_budget = summary
        .reasoning
        .budget_tokens
        .map(|tokens| tokens.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let reasoning_enabled = if summary.reasoning.enabled {
        "on"
    } else {
        "off"
    };
    let modules = summary.modules.clone();
    let enabled_tools = summary.tools_enabled.clone();
    let registered_tools = summary.registered_tools.clone();
    let plugins = summary.plugins.clone();
    let config_files = summary.config_files.clone();
    let model_options = summary.model_options.clone();

    view! {
        <div class="configs-scroll">
            <section class="config-overview">
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"runtime"</span>
                        <strong>{non_empty(summary.profile.as_str(), "default")}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"cwd"</span>
                        <code>{non_empty(summary.cwd.as_str(), "-")}</code>
                    </div>
                    <div class="config-kv">
                        <span>"config"</span>
                        <code>{config_path}</code>
                    </div>
                    <div class="config-kv">
                        <span>"mode"</span>
                        <code>{non_empty(summary.permission_mode.as_str(), "-")}</code>
                    </div>
                </article>
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"model"</span>
                        <strong>{model_label}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"provider"</span>
                        <code>{non_empty(summary.model.provider.as_str(), "-")}</code>
                    </div>
                    <div class="config-kv">
                        <span>"name"</span>
                        <code>{non_empty(summary.model.name.as_str(), "-")}</code>
                    </div>
                    <div class="config-chip-row">
                        <For
                            each=move || model_options.clone()
                            key=|model| model.label.clone()
                            children=move |model| view! { <span class="config-chip">{non_empty(model.label.as_str(), model.name.as_str())}</span> }
                        />
                    </div>
                </article>
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"reasoning"</span>
                        <strong>{reasoning_enabled}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"effort"</span>
                        <code>{reasoning_effort}</code>
                    </div>
                    <div class="config-kv">
                        <span>"summary"</span>
                        <code>{if summary.reasoning.summary { "true" } else { "false" }}</code>
                    </div>
                    <div class="config-kv">
                        <span>"budget"</span>
                        <code>{reasoning_budget}</code>
                    </div>
                    <div class="config-chip-row">
                        <For
                            each=move || summary.reasoning.effort_options.clone()
                            key=|effort| effort.clone()
                            children=move |effort| view! { <span class="config-chip">{effort}</span> }
                        />
                    </div>
                </article>
            </section>

            {builder
                .map(|builder| {
                    view! {
                        <ConfigBuilderView
                            builder
                            draft_modules
                            set_draft_modules
                            draft_config_texts
                            set_draft_config_texts
                            draft_module_config
                            set_draft_module_config
                            set_builder
                            set_summary
                            set_status
                        />
                    }
                    .into_any()
                })
                .unwrap_or_else(|| {
                    view! {
                        <section class="config-section">
                            <div class="config-section-header">
                                <h3>"Config builder"</h3>
                                <span>"offline"</span>
                            </div>
                            <div class="config-empty">"Builder endpoint недоступен"</div>
                        </section>
                    }
                    .into_any()
                })}

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Modules"</h3>
                    <span>{modules.len()}</span>
                </div>
                <div class="config-table">
                    <For
                        each=move || modules.clone()
                        key=|module| format!("{}:{}", module.slot, module.id)
                        children=move |module| {
                            view! {
                                <div class="config-row">
                                    <span>{module.slot}</span>
                                    <code>{module.id}</code>
                                </div>
                            }
                        }
                    />
                </div>
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Enabled tools"</h3>
                    <span>{enabled_tools.len()}</span>
                </div>
                {if enabled_tools.is_empty() {
                    view! { <div class="config-empty">"(none)"</div> }.into_any()
                } else {
                    view! {
                        <div class="config-chip-row">
                            <For
                                each=move || enabled_tools.clone()
                                key=|tool| tool.clone()
                                children=move |tool| view! { <span class="config-chip">{tool}</span> }
                            />
                        </div>
                    }.into_any()
                }}
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Registered tools"</h3>
                    <span>{registered_tools.len()}</span>
                </div>
                <div class="config-list">
                    <For
                        each=move || registered_tools.clone()
                        key=|tool| format!("{}:{}", tool.name, tool.source)
                        children=move |tool| {
                            view! {
                                <article class="config-list-item">
                                    <div class="config-list-main">
                                        <div class="config-list-title">
                                            <strong>{tool.name}</strong>
                                            <code>{tool.source}</code>
                                        </div>
                                        <p>{tool.description}</p>
                                    </div>
                                    <span class="status-badge idle">{tool.safety}</span>
                                </article>
                            }
                        }
                    />
                </div>
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Plugins"</h3>
                    <span>{plugins.len()}</span>
                </div>
                <div class="config-list">
                    <For
                        each=move || plugins.clone()
                        key=|plugin| format!("{}:{}", plugin.name, plugin.version)
                        children=move |plugin| {
                            let badge_class = if plugin.status.starts_with("error") {
                                "status-badge failed"
                            } else {
                                "status-badge completed"
                            };
                            view! {
                                <article class="config-list-item">
                                    <div class="config-list-main">
                                        <div class="config-list-title">
                                            <strong>{plugin.name}</strong>
                                            <code>{plugin.version}</code>
                                        </div>
                                        <p>{plugin.description}</p>
                                    </div>
                                    <span class=badge_class>
                                        <span class="dot"></span>
                                        {plugin.status}
                                    </span>
                                </article>
                            }
                        }
                    />
                </div>
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Config files"</h3>
                    <span>{config_files.len()}</span>
                </div>
                {if config_files.is_empty() {
                    view! { <div class="config-empty">"(none)"</div> }.into_any()
                } else {
                    view! {
                        <div class="config-table">
                            <For
                                each=move || config_files.clone()
                                key=|path| path.clone()
                                children=move |path| {
                                    view! {
                                        <div class="config-row">
                                            <span>{short_path(&path)}</span>
                                            <code>{path}</code>
                                        </div>
                                    }
                                }
                            />
                        </div>
                    }.into_any()
                }}
            </section>
        </div>
    }
}

#[component]
fn ConfigBuilderView(
    builder: ConfigBuilderSnapshot,
    draft_modules: ReadSignal<BTreeMap<String, String>>,
    set_draft_modules: WriteSignal<BTreeMap<String, String>>,
    draft_config_texts: ReadSignal<BTreeMap<String, String>>,
    set_draft_config_texts: WriteSignal<BTreeMap<String, String>>,
    draft_module_config: ReadSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    set_draft_module_config: WriteSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    set_builder: WriteSignal<Option<ConfigBuilderSnapshot>>,
    set_summary: WriteSignal<Option<ConfigSummary>>,
    set_status: WriteSignal<String>,
) -> impl IntoView {
    let slots = builder.slots.clone();
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
            };
            match post_json::<_, ConfigBuilderSnapshot>("/config/builder", &request).await {
                Ok(next_builder) => {
                    let next_modules = builder_active_modules(&next_builder);
                    let next_texts = builder_config_texts(&next_builder, &next_modules);
                    set_draft_module_config.set(next_builder.module_config.clone());
                    set_draft_modules.set(next_modules);
                    set_draft_config_texts.set(next_texts);
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
                        let slot_id = slot.id.clone();
                        let slot_id_for_select_value = slot_id.clone();
                        let slot_id_for_select_change = slot_id.clone();
                        let slot_id_for_text_value = slot_id.clone();
                        let slot_id_for_text_input = slot_id.clone();
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
                                <label class="config-builder-field">
                                    <span>"module_config JSON"</span>
                                    <textarea
                                        spellcheck="false"
                                        prop:value=move || {
                                            draft_config_texts
                                                .with(|items| items.get(&slot_id_for_text_value).cloned())
                                                .unwrap_or_else(|| "{}".to_owned())
                                        }
                                        on:input:target=move |ev| {
                                            set_draft_config_texts.update(|items| {
                                                items.insert(slot_id_for_text_input.clone(), ev.target().value());
                                            });
                                        }
                                    ></textarea>
                                </label>
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
                />
            </div>
        </section>
    }
}

fn builder_active_modules(builder: &ConfigBuilderSnapshot) -> BTreeMap<String, String> {
    builder
        .active_modules
        .iter()
        .map(|module| (module.slot.clone(), module.id.clone()))
        .collect()
}

fn builder_config_texts(
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

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}
