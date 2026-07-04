use leptos::prelude::*;

use crate::types::*;
use crate::ui_utils::{short_path, shorten_home};

/// Read-only панели runtime/model/reasoning над builder-ом. Здесь только
/// текущее runtime-состояние; всё редактируемое (modules, provider, mode,
/// tools) живёт в Config builder и не дублируется списками ниже.
#[component]
pub(super) fn ConfigOverview(summary: ConfigSummary) -> impl IntoView {
    let model_label = non_empty(summary.model.label.as_str(), "model не выбран");
    let config_path_full = summary
        .config_path
        .as_deref()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "(default discovery / none)".to_owned());
    let config_path = shorten_home(&config_path_full);
    let cwd_full = summary.cwd.clone();
    let cwd = shorten_home(non_empty(summary.cwd.as_str(), "-").as_str());
    let config_files = summary.config_files.clone();
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

    view! {
        <section class="config-overview">
            <article class="config-panel">
                <div class="config-panel-header">
                    <span class="panel-kicker">"runtime"</span>
                    <strong>{non_empty(summary.profile.as_str(), "default")}</strong>
                </div>
                <div class="config-kv">
                    <span>"cwd"</span>
                    <code title=cwd_full>{cwd}</code>
                </div>
                <div class="config-kv">
                    <span>"config"</span>
                    <code title=config_path_full>{config_path}</code>
                </div>
                <div class="config-kv">
                    <span>"mode"</span>
                    <code>{non_empty(summary.permission_mode.as_str(), "-")}</code>
                </div>
                {(!config_files.is_empty())
                    .then(|| {
                        view! {
                            <div class="config-chip-row">
                                <For
                                    each=move || config_files.clone()
                                    key=|path| path.clone()
                                    children=move |path| {
                                        let title = path.clone();
                                        view! {
                                            <span class="config-chip" title=title>{short_path(&path)}</span>
                                        }
                                    }
                                />
                            </div>
                        }
                    })}
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
            </article>
        </section>
    }
}

/// Read-only секция Plugins под builder-ом. Modules/tools/config files здесь
/// не дублируются: modules и tools редактируются в builder, config files
/// показаны в overview.
#[component]
pub(super) fn ConfigSections(summary: ConfigSummary) -> impl IntoView {
    let plugins = summary.plugins.clone();

    view! {
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
    }
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}
