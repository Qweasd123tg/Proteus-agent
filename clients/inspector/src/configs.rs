use std::collections::{BTreeMap, BTreeSet};

use leptos::{prelude::*, task::spawn_local};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::BeforeUnloadEvent;

use crate::api::get_json;
use crate::types::*;
use crate::ui_utils::short_path;

mod builder;
mod draft;
mod module_config_editor;
mod summary;
mod tools_picker;

use builder::{ConfigBuilderView, ModuleDrafts, builder_active_modules, builder_config_texts};
use summary::{ConfigOverview, ConfigSections};

pub(super) type DraftErrors = BTreeMap<String, String>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum SaveFeedback {
    #[default]
    Ready,
    Saved,
    Error(String),
}

#[component]
pub(crate) fn ConfigsView() -> impl IntoView {
    let (summary, set_summary) = signal(None::<ConfigSummary>);
    let (builder, set_builder) = signal(None::<ConfigBuilderSnapshot>);
    let (draft_modules, set_draft_modules) = signal(BTreeMap::<String, String>::new());
    let (draft_config_texts, set_draft_config_texts) = signal(ModuleDrafts::new());
    let (draft_errors, set_draft_errors) = signal(DraftErrors::new());
    let (draft_tools, set_draft_tools) = signal(BTreeSet::<String>::new());
    let (draft_provider, set_draft_provider) = signal(String::new());
    let (draft_mode, set_draft_mode) = signal(String::new());
    let (status, set_status) = signal("Загружаю конфигурацию…".to_owned());
    let saving = RwSignal::new(false);
    let loading = RwSignal::new(false);
    let busy = Signal::derive(move || saving.get() || loading.get());
    let save_feedback = RwSignal::new(SaveFeedback::Ready);

    let drafts = DraftSetters {
        modules: set_draft_modules,
        config_texts: set_draft_config_texts,
        errors: set_draft_errors,
        tools: set_draft_tools,
        provider: set_draft_provider,
        mode: set_draft_mode,
    };

    let dirty = Memo::new(move |_| {
        builder.with(|builder| {
            builder.as_ref().is_some_and(|builder| {
                !draft_errors.get().is_empty()
                    || is_dirty(
                        builder,
                        &draft_modules.get(),
                        &draft_config_texts.get(),
                        &draft_tools.get(),
                        &draft_provider.get(),
                        &draft_mode.get(),
                    )
            })
        })
    });

    let before_unload =
        Closure::<dyn FnMut(BeforeUnloadEvent)>::new(move |event: BeforeUnloadEvent| {
            if dirty.get_untracked() || saving.get_untracked() {
                event.prevent_default();
                event.set_return_value("");
            }
        });
    if let Some(window) = web_sys::window() {
        let _ = window.add_event_listener_with_callback(
            "beforeunload",
            before_unload.as_ref().unchecked_ref(),
        );
        before_unload.forget();
    }

    load_config_page(
        set_summary,
        set_builder,
        drafts,
        set_status,
        save_feedback,
        loading,
    );

    let refresh = move |_| {
        if busy.get_untracked() || dirty.get_untracked() {
            return;
        }
        load_config_page(
            set_summary,
            set_builder,
            drafts,
            set_status,
            save_feedback,
            loading,
        );
    };

    view! {
        <section class="configs-page">
            <header class="cfg-page-head">
                <div>
                    <span class="panel-kicker">"КОНСТРУКТОР АГЕНТОВ"</span>
                    <h2>"Сборка агента"</h2>
                    <p>"Выберите модули, инструменты и режим запуска текущего профиля."</p>
                </div>
                <div class="cfg-page-actions">
                    <span class="topology-muted">{move || status.get()}</span>
                    <button
                        type="button"
                        class="secondary"
                        disabled=move || busy.get() || dirty.get()
                        title=move || if dirty.get() {
                            "Сначала сохраните или сбросьте изменения"
                        } else {
                            "Обновить данные"
                        }
                        on:click=refresh
                    >"Обновить"</button>
                </div>
            </header>
            {move || summary.get().map(|summary| {
                view! {
                    <div class="configs-scroll">
                        <ConfigOverview summary=summary.clone()/>
                        {move || builder.get().map(|builder| {
                            view! {
                                <ConfigBuilderView
                                    builder
                                    summary=summary.clone()
                                    draft_modules
                                    draft_config_texts
                                    draft_errors
                                    draft_tools
                                    draft_provider
                                    draft_mode
                                    drafts
                                    dirty
                                    saving
                                    busy
                                    save_feedback
                                    set_builder
                                    set_summary
                                    set_status
                                />
                            }.into_any()
                        }).unwrap_or_else(|| view! {
                            <div class="cfg-panel">
                                <div class="config-empty">"Редактор сборки недоступен. Данные текущей сборки доступны ниже."</div>
                                <ConfigSections summary=summary.clone()/>
                            </div>
                        }.into_any())}
                    </div>
                }.into_any()
            }).unwrap_or_else(|| view! {
                <div class="empty-state">
                    <div class="empty-state-title">"Конфигурация недоступна"</div>
                </div>
            }.into_any())}
        </section>
    }
}

#[derive(Clone, Copy)]
pub(super) struct DraftSetters {
    modules: WriteSignal<BTreeMap<String, String>>,
    config_texts: WriteSignal<ModuleDrafts>,
    errors: WriteSignal<DraftErrors>,
    tools: WriteSignal<BTreeSet<String>>,
    provider: WriteSignal<String>,
    mode: WriteSignal<String>,
}

impl DraftSetters {
    pub(super) fn reset_to(&self, builder: &ConfigBuilderSnapshot) {
        self.modules.set(builder_active_modules(builder));
        self.config_texts.set(builder_config_texts(builder));
        self.errors.set(DraftErrors::new());
        self.tools
            .set(builder.tools_enabled.iter().cloned().collect());
        self.provider.set(builder.active_provider.clone());
        self.mode.set(builder.permission_mode.clone());
    }
}

fn is_dirty(
    builder: &ConfigBuilderSnapshot,
    modules: &BTreeMap<String, String>,
    config_texts: &ModuleDrafts,
    tools: &BTreeSet<String>,
    provider: &str,
    mode: &str,
) -> bool {
    *modules != builder_active_modules(builder)
        || *config_texts != builder_config_texts(builder)
        || *tools != builder.tools_enabled.iter().cloned().collect()
        || provider != builder.active_provider
        || mode != builder.permission_mode
}

fn load_config_page(
    set_summary: WriteSignal<Option<ConfigSummary>>,
    set_builder: WriteSignal<Option<ConfigBuilderSnapshot>>,
    drafts: DraftSetters,
    set_status: WriteSignal<String>,
    save_feedback: RwSignal<SaveFeedback>,
    loading: RwSignal<bool>,
) {
    if loading.get_untracked() {
        return;
    }
    loading.set(true);
    set_status.set("Загружаю конфигурацию…".to_owned());
    spawn_local(async move {
        match get_json::<ConfigSummary>("/config").await {
            Ok(summary) => set_summary.set(Some(summary)),
            Err(error) => {
                set_summary.set(None);
                set_builder.set(None);
                set_status.set(format!("Не удалось загрузить конфигурацию: {error}"));
                loading.set(false);
                return;
            }
        }

        match get_json::<ConfigBuilderSnapshot>("/config/builder").await {
            Ok(builder) => {
                drafts.reset_to(&builder);
                let target = builder
                    .target_path
                    .as_deref()
                    .map(short_path)
                    .unwrap_or_else(|| "без файла".to_owned());
                set_builder.set(Some(builder));
                set_status.set(format!("Профиль загружен · {target}"));
                save_feedback.set(SaveFeedback::Ready);
            }
            Err(error) => {
                set_builder.set(None);
                set_status.set(format!("Сводка загружена, редактор недоступен: {error}"));
            }
        }
        loading.set(false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_tracks_per_module_raw_drafts() {
        let mut builder = ConfigBuilderSnapshot::default();
        builder.active_modules.push(ConfigModule {
            slot: "model".into(),
            id: "a".into(),
            ..Default::default()
        });
        builder.module_config.insert(
            "model".into(),
            BTreeMap::from([("a".into(), serde_json::json!({"x": 1}))]),
        );
        let modules = builder_active_modules(&builder);
        let mut texts = builder_config_texts(&builder);
        let tools = BTreeSet::new();
        assert!(!is_dirty(&builder, &modules, &texts, &tools, "", ""));
        texts
            .entry("model".into())
            .or_default()
            .insert("a".into(), "{ invalid".into());
        assert!(is_dirty(&builder, &modules, &texts, &tools, "", ""));
    }
}
