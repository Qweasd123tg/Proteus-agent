use leptos::prelude::*;

use crate::types::*;
use crate::ui_utils::short_path;

/// Read-only панели runtime/model/reasoning над builder-ом.
#[component]
pub(super) fn ConfigOverview(summary: ConfigSummary) -> impl IntoView {
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
    let model_options = summary.model_options.clone();

    view! {
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
    }
}

/// Read-only секции Modules/Plugins/Config files под builder-ом. Tools здесь
/// не дублируются: их каталог и переключение живут в builder ToolsPicker.
#[component]
pub(super) fn ConfigSections(summary: ConfigSummary) -> impl IntoView {
    let modules = summary.modules.clone();
    let plugins = summary.plugins.clone();
    let config_files = summary.config_files.clone();

    view! {
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
    }
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}
