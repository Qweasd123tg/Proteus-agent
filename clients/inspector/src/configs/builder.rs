use std::collections::{BTreeMap, BTreeSet};

use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;

use crate::api::{get_json, post_json};
use crate::types::*;
use crate::ui_utils::shorten_home;

use super::DraftSetters;
use super::module_config_editor::ModuleConfigEditor;
use super::tools_picker::ToolsPicker;

#[component]
pub(super) fn ConfigBuilderView(
    builder: ConfigBuilderSnapshot,
    draft_modules: ReadSignal<BTreeMap<String, String>>,
    draft_config_texts: ReadSignal<BTreeMap<String, String>>,
    draft_module_config: ReadSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    draft_tools: ReadSignal<BTreeSet<String>>,
    draft_provider: ReadSignal<Option<String>>,
    draft_mode: ReadSignal<String>,
    drafts: DraftSetters,
    set_builder: WriteSignal<Option<ConfigBuilderSnapshot>>,
    set_summary: WriteSignal<Option<ConfigSummary>>,
    set_status: WriteSignal<String>,
) -> impl IntoView {
    let slots = builder.slots.clone();
    let tools = builder.tools.clone();
    let warnings = builder.warnings.clone();
    let target_path_full = builder
        .target_path
        .clone()
        .unwrap_or_else(|| "(config path unavailable)".to_owned());
    let target_path = shorten_home(&target_path_full);
    let writable = builder.writable;

    // Dirty-состояние: сравнение черновиков со snapshot-ом. Ложное
    // срабатывание из-за форматирования JSON безвредно (кнопка просто
    // активна), пропуск реального изменения невозможен — сравнение точное.
    let saved_modules = builder_active_modules(&builder);
    let saved_texts = builder_config_texts(&builder, &saved_modules);
    let saved_tools = builder.tools_enabled.iter().cloned().collect::<BTreeSet<_>>();
    let saved_provider = builder.active_provider.clone();
    let saved_mode = builder.permission_mode.clone();
    let dirty = Memo::new(move |_| {
        draft_modules.with(|modules| *modules != saved_modules)
            || draft_config_texts.with(|texts| *texts != saved_texts)
            || draft_tools.with(|tools| *tools != saved_tools)
            || draft_provider.with(|provider| *provider != saved_provider)
            || draft_mode.with(|mode| *mode != saved_mode)
    });

    let save = move |_| {
        if !writable {
            set_status.set("config path недоступен для записи".to_owned());
            return;
        }
        let modules = draft_modules.get_untracked();
        let mut module_config = draft_module_config.get_untracked();
        let text_by_slot = draft_config_texts.get_untracked();
        let tools_enabled = draft_tools.get_untracked().into_iter().collect::<Vec<_>>();
        let active_provider = draft_provider.get_untracked();
        let permission_mode = Some(draft_mode.get_untracked()).filter(|mode| !mode.is_empty());
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
                active_provider,
                permission_mode,
            };
            match post_json::<_, ConfigBuilderSnapshot>("/config/builder", &request).await {
                Ok(next_builder) => {
                    drafts.reset_to(&next_builder);
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
                <code title=target_path_full>{target_path}</code>
                <span class="topology-muted">
                    {move || if dirty.get() { "есть несохранённые изменения" } else { "" }}
                </span>
                <button
                    type="button"
                    class="btn-primary"
                    disabled=move || !writable || !dirty.get()
                    on:click=save
                >
                    "Сохранить"
                </button>
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
            <RuntimeSettings builder=builder.clone() draft_provider draft_mode drafts/>
            <div class="config-builder-grid">
                <For
                    each=move || slots.clone()
                    key=|slot| slot.id.clone()
                    children=move |slot| {
                        view! {
                            <BuilderSlotCard
                                builder_slot=slot
                                draft_modules
                                draft_config_texts
                                draft_module_config
                                drafts
                            />
                        }
                    }
                />
            </div>
            <ToolsPicker tools draft_tools set_draft_tools=drafts.tools/>
        </section>
    }
}

/// Provider (модель) и permission mode: config-уровневые настройки за
/// пределами module slots. Provider доступен только когда в config есть
/// `[providers]`; иначе модель задана секцией `[model]` и выбор недоступен.
#[component]
fn RuntimeSettings(
    builder: ConfigBuilderSnapshot,
    draft_provider: ReadSignal<Option<String>>,
    draft_mode: ReadSignal<String>,
    drafts: DraftSetters,
) -> impl IntoView {
    let providers = builder.providers.clone();
    let modes = builder.permission_modes.clone();

    view! {
        <div class="config-builder-grid config-builder-runtime">
            <article class="config-builder-slot">
                <div class="config-builder-slot-head">
                    <div>
                        <span class="panel-kicker">"config"</span>
                        <strong>"Model provider"</strong>
                    </div>
                    <code>"active_provider"</code>
                </div>
                <p>"Активный provider из [providers]; save переключает модель и перезагружает runtime."</p>
                {if providers.is_empty() {
                    view! {
                        <div class="config-empty">"[providers] не настроены — модель задаётся секцией [model]"</div>
                    }.into_any()
                } else {
                    let options = providers.clone();
                    view! {
                        <label class="config-builder-field">
                            <span>"provider"</span>
                            <select
                                prop:value=move || draft_provider.get().unwrap_or_default()
                                on:change:target=move |ev| {
                                    drafts.provider.set(Some(ev.target().value()));
                                }
                            >
                                <For
                                    each=move || options.clone()
                                    key=|provider| provider.id.clone()
                                    children=move |provider| {
                                        view! {
                                            <option value=provider.id.clone()>
                                                {format!("{} · {}", provider.id, provider.label)}
                                            </option>
                                        }
                                    }
                                />
                            </select>
                        </label>
                    }.into_any()
                }}
            </article>
            <article class="config-builder-slot">
                <div class="config-builder-slot-head">
                    <div>
                        <span class="panel-kicker">"config"</span>
                        <strong>"Permission mode"</strong>
                    </div>
                    <code>"permissions.mode"</code>
                </div>
                <p>"plan — только чтение, normal — approvals по policy, auto — авто-подтверждение."</p>
                <label class="config-builder-field">
                    <span>"mode"</span>
                    <select
                        prop:value=move || draft_mode.get()
                        on:change:target=move |ev| drafts.mode.set(ev.target().value())
                    >
                        <For
                            each=move || modes.clone()
                            key=|mode| mode.clone()
                            children=move |mode| {
                                view! { <option value=mode.clone()>{mode.clone()}</option> }
                            }
                        />
                    </select>
                </label>
            </article>
        </div>
    }
}

#[component]
fn BuilderSlotCard(
    builder_slot: ConfigBuilderSlot,
    draft_modules: ReadSignal<BTreeMap<String, String>>,
    draft_config_texts: ReadSignal<BTreeMap<String, String>>,
    draft_module_config: ReadSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    drafts: DraftSetters,
) -> impl IntoView {
    let slot = builder_slot;
    let slot_id = slot.id.clone();
    let slot_id_for_select_value = slot_id.clone();
    let slot_id_for_select_change = slot_id.clone();
    let modules_for_select = slot.modules.clone();

    // Выбранный модуль (реактивно по черновику): его описание и capabilities
    // показываются под select-ом — карточка объясняет, что именно выбрано.
    let modules_for_details = slot.modules.clone();
    let details_slot_id = slot_id.clone();
    let selected_module = Memo::new(move |_| {
        let active = draft_modules
            .with(|items| items.get(&details_slot_id).cloned())
            .unwrap_or_default();
        modules_for_details
            .iter()
            .find(|module| module.id == active)
            .cloned()
    });

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
                <span>"module"</span>
                <select
                    prop:value=move || {
                        draft_modules
                            .with(|items| items.get(&slot_id_for_select_value).cloned())
                            .unwrap_or_default()
                    }
                    on:change:target=move |ev| {
                        let selected = ev.target().value();
                        drafts.modules.update(|items| {
                            items.insert(slot_id_for_select_change.clone(), selected.clone());
                        });
                        let text = draft_module_config.with(|config| {
                            module_config_text(config, &slot_id_for_select_change, &selected)
                        });
                        drafts.config_texts.update(|items| {
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
            <div class="config-builder-modules">
                <p class="config-builder-module-note">
                    {move || {
                        selected_module
                            .get()
                            .and_then(|module| module.description)
                            .filter(|description| !description.trim().is_empty())
                            .unwrap_or_else(|| "(описание модуля не задано)".to_owned())
                    }}
                </p>
                <div class="config-chip-row">
                    <For
                        each=move || {
                            selected_module
                                .get()
                                .map(|module| module.capabilities)
                                .unwrap_or_default()
                        }
                        key=|capability| capability.clone()
                        children=move |capability| view! { <span class="config-chip">{capability}</span> }
                    />
                </div>
            </div>
            <ModuleConfigEditor
                slot_id=slot_id.clone()
                draft_config_texts
                set_draft_config_texts=drafts.config_texts
            />
        </article>
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
