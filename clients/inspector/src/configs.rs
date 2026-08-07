use std::collections::{BTreeMap, BTreeSet};

use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;

use crate::api::get_json;
use crate::types::*;
use crate::ui_utils::short_path;

mod builder;
mod module_config_editor;
mod summary;
mod tools_picker;

use builder::{ConfigBuilderView, builder_active_modules, builder_config_texts};
use summary::{ConfigOverview, ConfigSections};

#[component]
pub(crate) fn ConfigsView() -> impl IntoView {
    let (summary, set_summary) = signal(None::<ConfigSummary>);
    let (builder, set_builder) = signal(None::<ConfigBuilderSnapshot>);
    let (draft_modules, set_draft_modules) = signal(BTreeMap::<String, String>::new());
    let (draft_config_texts, set_draft_config_texts) = signal(BTreeMap::<String, String>::new());
    let (draft_module_config, set_draft_module_config) =
        signal(BTreeMap::<String, BTreeMap<String, Value>>::new());
    let (draft_tools, set_draft_tools) = signal(BTreeSet::<String>::new());
    let (draft_provider, set_draft_provider) = signal(String::new());
    let (draft_mode, set_draft_mode) = signal(String::new());
    let (status, set_status) = signal("загружаю конфигурацию".to_owned());

    let drafts = DraftSetters {
        modules: set_draft_modules,
        config_texts: set_draft_config_texts,
        module_config: set_draft_module_config,
        tools: set_draft_tools,
        provider: set_draft_provider,
        mode: set_draft_mode,
    };

    load_config_page(set_summary, set_builder, drafts, set_status);

    let refresh = move |_| load_config_page(set_summary, set_builder, drafts, set_status);

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
                            <div class="configs-scroll">
                                <ConfigOverview summary=summary.clone()/>
                                {move || {
                                    builder
                                        .get()
                                        .map(|builder| {
                                            view! {
                                                <ConfigBuilderView
                                                    builder
                                                    draft_modules
                                                    draft_config_texts
                                                    draft_module_config
                                                    draft_tools
                                                    draft_provider
                                                    draft_mode
                                                    drafts
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
                                        })
                                }}
                                <ConfigSections summary/>
                            </div>
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

/// Общий пучок write-сигналов черновика builder-а: черновик сбрасывается в
/// состояние snapshot-а при загрузке страницы и после успешного save.
#[derive(Clone, Copy)]
struct DraftSetters {
    modules: WriteSignal<BTreeMap<String, String>>,
    config_texts: WriteSignal<BTreeMap<String, String>>,
    module_config: WriteSignal<BTreeMap<String, BTreeMap<String, Value>>>,
    tools: WriteSignal<BTreeSet<String>>,
    provider: WriteSignal<String>,
    mode: WriteSignal<String>,
}

impl DraftSetters {
    /// Приводит все черновики к состоянию свежего snapshot-а.
    fn reset_to(&self, builder: &ConfigBuilderSnapshot) {
        let modules = builder_active_modules(builder);
        let texts = builder_config_texts(builder, &modules);
        self.module_config.set(builder.module_config.clone());
        self.modules.set(modules);
        self.config_texts.set(texts);
        self.tools.set(builder.tools_enabled.iter().cloned().collect());
        self.provider.set(builder.active_provider.clone());
        self.mode.set(builder.permission_mode.clone());
    }
}

fn load_config_page(
    set_summary: WriteSignal<Option<ConfigSummary>>,
    set_builder: WriteSignal<Option<ConfigBuilderSnapshot>>,
    drafts: DraftSetters,
    set_status: WriteSignal<String>,
) {
    spawn_local(async move {
        match get_json::<ConfigSummary>("/config").await {
            Ok(summary) => {
                let module_count = summary.modules.len();
                let tool_count = summary.registered_tools.len();
                let process_count = summary.process_modules.len();
                set_summary.set(Some(summary));
                set_status.set(format!(
                    "{module_count} selected modules · {tool_count} tools · {process_count} process workers"
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
                drafts.reset_to(&builder);
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
