use std::collections::{BTreeMap, BTreeSet};

use leptos::{prelude::*, task::spawn_local};

use crate::api::{get_json, post_json};
use crate::types::*;
use crate::ui_utils::shorten_home;

use super::module_config_editor::{ModuleConfigEditor, has_module_errors};
use super::summary::ConfigSections;
use super::tools_picker::ToolsPicker;
use super::{DraftErrors, DraftSetters, SaveFeedback};

pub(super) use super::draft::{ModuleDrafts, builder_active_modules, builder_config_texts};
use super::draft::{
    filter_slots, parse_module_drafts, permission_label, save_state_class, save_state_text,
    slot_presentation,
};

#[component]
pub(super) fn ConfigBuilderView(
    builder: ConfigBuilderSnapshot,
    summary: ConfigSummary,
    draft_modules: ReadSignal<BTreeMap<String, String>>,
    draft_config_texts: ReadSignal<ModuleDrafts>,
    draft_errors: ReadSignal<DraftErrors>,
    draft_tools: ReadSignal<BTreeSet<String>>,
    draft_provider: ReadSignal<String>,
    draft_mode: ReadSignal<String>,
    drafts: DraftSetters,
    dirty: Memo<bool>,
    saving: RwSignal<bool>,
    busy: Signal<bool>,
    save_feedback: RwSignal<SaveFeedback>,
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
        .unwrap_or_else(|| "(путь конфигурации недоступен)".to_owned());
    let target_path = shorten_home(&target_path_full);
    let writable = builder.writable;
    let active_tab = RwSignal::new("modules".to_owned());
    let slot_search = RwSignal::new(String::new());
    let saved_module_config = builder.module_config.clone();
    let validation_baseline = saved_module_config.clone();
    let valid_json = Memo::new(move |_| {
        parse_module_drafts(&draft_config_texts.get(), &validation_baseline).is_ok()
    });

    let builder_for_reset = builder.clone();
    let reset = move |_| {
        if busy.get_untracked() {
            return;
        }
        drafts.reset_to(&builder_for_reset);
        set_builder.set(Some(builder_for_reset.clone()));
        save_feedback.set(SaveFeedback::Ready);
        set_status.set("Изменения сброшены".to_owned());
    };

    let save = move |_| {
        if busy.get_untracked() || !writable || !dirty.get_untracked() {
            return;
        }
        if !draft_errors.get_untracked().is_empty() {
            save_feedback.set(SaveFeedback::Error(
                "Исправьте значения параметров".to_owned(),
            ));
            return;
        }
        let module_config =
            match parse_module_drafts(&draft_config_texts.get_untracked(), &saved_module_config) {
                Ok(config) => config,
                Err(error) => {
                    save_feedback.set(SaveFeedback::Error(error));
                    return;
                }
            };
        let request = ConfigBuilderSaveRequest {
            modules: draft_modules.get_untracked(),
            module_config,
            tools_enabled: Some(draft_tools.get_untracked().into_iter().collect()),
            active_provider: Some(draft_provider.get_untracked()),
            permission_mode: Some(draft_mode.get_untracked())
                .filter(|mode| !mode.is_empty())
                .map(|mode| {
                    serde_json::from_value(serde_json::Value::String(mode))
                        .expect("selected permission mode")
                }),
        };
        saving.set(true);
        set_status.set("Сохраняю сборку…".to_owned());
        spawn_local(async move {
            match post_json::<_, ConfigBuilderSnapshot>("/config/builder", &request).await {
                Ok(next_builder) => {
                    drafts.reset_to(&next_builder);
                    set_builder.set(Some(next_builder));
                    match get_json::<ConfigSummary>("/config").await {
                        Ok(summary) => set_summary.set(Some(summary)),
                        Err(error) => {
                            save_feedback.set(SaveFeedback::Error(format!(
                                "Сборка сохранена, сводка не обновилась: {error}"
                            )));
                            set_status.set("Сборка сохранена".to_owned());
                            saving.set(false);
                            return;
                        }
                    }
                    save_feedback.set(SaveFeedback::Saved);
                    set_status.set("Сборка сохранена · runtime перезагружен".to_owned());
                }
                Err(error) => {
                    save_feedback.set(SaveFeedback::Error(format!(
                        "Не удалось сохранить: {error}"
                    )));
                    set_status.set("Ошибка сохранения".to_owned());
                }
            }
            saving.set(false);
        });
    };

    let filter_catalog = slots.clone();
    let filtered_slots = Memo::new(move |_| filter_slots(&filter_catalog, &slot_search.get()));

    view! {
        <section class="config-section config-builder cfg-workspace">
            <RuntimeSettings builder=builder.clone() draft_provider draft_mode drafts busy/>
            <nav class="cfg-tabs" aria-label="Разделы сборки">
                <For
                    each=move || [("modules", "Модули"), ("tools", "Инструменты"), ("details", "Детали")]
                    key=|(id, _)| *id
                    children=move |(id, label)| view! {
                        <button
                            type="button"
                            class="cfg-tab"
                            class:active=move || active_tab.get() == id
                            aria-pressed=move || (active_tab.get() == id).to_string()
                            disabled=move || busy.get()
                            on:click=move |_| active_tab.set(id.to_owned())
                        >{label}</button>
                    }
                />
            </nav>
            <fieldset class="cfg-editor-surface" disabled=move || busy.get()>
                <section class="cfg-panel cfg-modules-panel" hidden=move || active_tab.get() != "modules">
                        <div class="cfg-panel-head">
                            <div><h3>"Модули"</h3><p>"Выберите реализацию для каждого слота."</p></div>
                            <input
                                class="cfg-search"
                                type="search"
                                placeholder="Поиск по названию, id или описанию"
                                prop:value=move || slot_search.get()
                                on:input:target=move |ev| slot_search.set(ev.target().value())
                            />
                        </div>
                        <div class="config-builder-grid">
                            <For
                                each=move || slots.clone()
                                key=|slot| slot.id.clone()
                                children=move |slot| {
                                    let filter_id = slot.id.clone();
                                    view! { <div class="cfg-slot" hidden=move || !filtered_slots.with(|slots| slots.iter().any(|slot| slot.id == filter_id))>
                                    <BuilderSlotCard
                                        builder_slot=slot
                                        draft_modules
                                        draft_config_texts
                                        draft_errors
                                        drafts
                                    />
                                    </div> }
                                }
                            />
                        </div>
                        <Show when=move || filtered_slots.get().is_empty()>
                            <div class="config-empty">"По этому запросу слоты не найдены"</div>
                        </Show>
                </section>
                <section class="cfg-panel" hidden=move || active_tab.get() != "tools"><ToolsPicker tools=tools.clone() draft_tools set_draft_tools=drafts.tools/></section>
                <section class="cfg-panel cfg-details-panel" hidden=move || active_tab.get() != "details">
                        <div class="config-builder-target">
                            <span>"Файл конфигурации"</span>
                            <code title=target_path_full.clone()>{target_path.clone()}</code>
                            <span>{if writable { "доступен для записи" } else { "только чтение" }}</span>
                        </div>
                        <Warnings warnings=warnings.clone()/>
                        <ConfigSections summary=summary.clone()/>
                </section>
            </fieldset>
            <div class="cfg-save-strip">
                <div class="cfg-save-info"><div role="status" aria-live="polite" class=move || save_state_class(saving.get(), dirty.get(), valid_json.get(), !draft_errors.get().is_empty(), &save_feedback.get())>
                    {move || save_state_text(saving.get(), dirty.get(), valid_json.get(), !draft_errors.get().is_empty(), &save_feedback.get())}
                </div><code title=target_path_full.clone()>{target_path.clone()}</code></div>
                <div class="cfg-save-actions">
                    <button type="button" class="secondary" disabled=move || busy.get() || !dirty.get() on:click=reset>"Сбросить"</button>
                    <button
                        type="button"
                        class="btn-primary"
                        disabled=move || busy.get() || !writable || !dirty.get() || !valid_json.get() || !draft_errors.get().is_empty()
                        on:click=save
                    >{move || if saving.get() { "Сохраняю…" } else { "Сохранить сборку" }}</button>
                </div>
            </div>
        </section>
    }
}

#[component]
fn RuntimeSettings(
    builder: ConfigBuilderSnapshot,
    draft_provider: ReadSignal<String>,
    draft_mode: ReadSignal<String>,
    drafts: DraftSetters,
    busy: Signal<bool>,
) -> impl IntoView {
    let providers = builder.providers.clone();
    let modes = builder.permission_modes.clone();
    view! {
        <div class="cfg-runtime-settings">
            <label class="config-builder-field">
                <span>"Активная модель"</span>
                <select disabled=move || busy.get() prop:value=move || draft_provider.get()
                    on:change:target=move |ev| drafts.provider.set(ev.target().value())>
                    <For each=move || providers.clone() key=|p| p.id.clone() children=move |p| view! {
                        <option value=p.id.clone()>{format!("{} · {}", p.label, p.id)}</option>
                    }/>
                </select>
            </label>
            <label class="config-builder-field">
                <span>"Режим разрешений"</span>
                <select disabled=move || busy.get() prop:value=move || draft_mode.get()
                    on:change:target=move |ev| drafts.mode.set(ev.target().value())>
                    <For each=move || modes.clone() key=|mode| mode.clone() children=move |mode| view! {
                        <option value=mode.clone()>{permission_label(&mode)}</option>
                    }/>
                </select>
            </label>
        </div>
    }
}

#[component]
fn BuilderSlotCard(
    builder_slot: ConfigBuilderSlot,
    draft_modules: ReadSignal<BTreeMap<String, String>>,
    draft_config_texts: ReadSignal<ModuleDrafts>,
    draft_errors: ReadSignal<DraftErrors>,
    drafts: DraftSetters,
) -> impl IntoView {
    let slot = builder_slot;
    let slot_id = slot.id.clone();
    let modules_for_select = slot.modules.clone();
    let modules_for_details = slot.modules.clone();
    let selected_slot = slot_id.clone();
    let selected_module = Memo::new(move |_| {
        let active = draft_modules
            .with(|items| items.get(&selected_slot).cloned())
            .unwrap_or_default();
        modules_for_details
            .iter()
            .find(|module| module.id == active)
            .cloned()
    });
    let select_slot = slot_id.clone();
    let change_slot = slot_id.clone();
    let errors_slot = slot_id.clone();
    let errors_module_slot = slot_id.clone();
    let has_errors = Memo::new(move |_| {
        let module = draft_modules
            .with(|items| items.get(&errors_module_slot).cloned())
            .unwrap_or_default();
        draft_errors.with(|errors| has_module_errors(errors, &errors_slot, &module))
    });
    let module_description = Signal::derive(move || {
        selected_module
            .get()
            .and_then(|module| module.description)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Дополнительное описание не задано".to_owned())
    });
    let module_capabilities = Signal::derive(move || {
        selected_module
            .get()
            .map(|module| module.capabilities)
            .unwrap_or_default()
    });
    let (display_title, display_responsibility) = slot_presentation(&slot);
    let title_slot = slot_id.clone();
    let editor_slot = slot_id.clone();
    view! {
        <article class="config-builder-slot">
            <div class="config-builder-slot-head">
                <div><span class="panel-kicker">{slot.category.clone()}</span><strong>{display_title}</strong></div>
                <code>{slot.id.clone()}</code>
            </div>
            <p>{display_responsibility}</p>
            <label class="config-builder-field">
                <span>"Реализация"</span>
                <select
                    disabled=move || has_errors.get()
                    title=move || if has_errors.get() { "Исправьте параметр перед сменой модуля" } else { "" }
                    prop:value=move || draft_modules.with(|items| items.get(&select_slot).cloned()).unwrap_or_default()
                    on:change:target=move |ev| {
                        let selected = ev.target().value();
                        drafts.modules.update(|items| { items.insert(change_slot.clone(), selected); });
                    }
                >
                    <For each=move || modules_for_select.clone() key=|module| module.id.clone() children=move |module| view! {
                        <option value=module.id.clone()>{module.id.clone()}</option>
                    }/>
                </select>
            </label>
            <ModuleConfigEditor
                slot_id=editor_slot
                module_id=Signal::derive(move || draft_modules.with(|items| items.get(&title_slot).cloned()).unwrap_or_default())
                module_description
                module_capabilities
                draft_config_texts
                draft_errors
                set_draft_config_texts=drafts.config_texts
                set_draft_errors=drafts.errors
            />
        </article>
    }
}

#[component]
fn Warnings(warnings: Vec<ConfigBuilderWarning>) -> impl IntoView {
    if warnings.is_empty() {
        return view! { <div></div> }.into_any();
    }
    view! { <div class="config-builder-warnings"><For each=move || warnings.clone() key=|w| format!("{}:{}", w.severity, w.message) children=move |w| view! {
        <div class="config-builder-warning"><span>{w.severity}</span><p>{w.message}</p></div>
    }/></div> }.into_any()
}
