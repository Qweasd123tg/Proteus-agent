use leptos::{prelude::*, task::spawn_local};

use crate::api::get_json;
use crate::types::*;
use crate::ui_utils::short_path;

#[component]
pub(crate) fn ConfigsView() -> impl IntoView {
    let (summary, set_summary) = signal(None::<ConfigSummary>);
    let (status, set_status) = signal("загружаю конфигурацию".to_owned());

    load_config_summary(set_summary, set_status);

    let refresh = move |_| load_config_summary(set_summary, set_status);

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
                    .map(|summary| view! { <ConfigSummaryView summary /> }.into_any())
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

fn load_config_summary(
    set_summary: WriteSignal<Option<ConfigSummary>>,
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
                set_status.set(format!("не удалось загрузить config: {error}"));
            }
        }
    });
}

#[component]
fn ConfigSummaryView(summary: ConfigSummary) -> impl IntoView {
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

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}
