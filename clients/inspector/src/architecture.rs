//! Вкладка Architecture: рендер `TopologySnapshot` из `/inspect/topology`.
//!
//! Принципы: каждый факт показывается один раз; группировка и порядок slots
//! приходят с сервера (`slot.category`/`slot.order`); никакой абсолютной
//! графики — pipeline рисуется потоком карточек; plugin contributions
//! берутся строго из `provides`, а их состояние вычисляется по
//! `modules`/`tools` snapshot-а.

use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;
use web_sys::MouseEvent;

use crate::api::{get_json, get_text};
use crate::architecture_map::{
    MapViewState, TopologyMapView, install_mermaid_rendered_fit, render_mermaid_map,
};
use crate::architecture_model::{
    backend_views, module_source_label, non_empty, pipeline_steps, plugin_contributions, slot_views,
};
use crate::types::*;
use crate::ui_utils::{compact_json, copy_to_clipboard};

#[component]
pub(crate) fn ArchitectureView() -> impl IntoView {
    let (snapshot, set_snapshot) = signal(None::<TopologySnapshot>);
    let (mermaid, set_mermaid) = signal(String::new());
    let (status, set_status) = signal("загружаю topology".to_owned());

    load_topology_snapshot(set_snapshot, set_mermaid, set_status);

    let refresh = move |_| load_topology_snapshot(set_snapshot, set_mermaid, set_status);
    let copy_mermaid = move |_| {
        let text = mermaid.get();
        if text.trim().is_empty() {
            set_status.set("Mermaid недоступен".to_owned());
        } else {
            copy_to_clipboard(text);
            set_status.set("Mermaid скопирован".to_owned());
        }
    };

    let map_view = MapViewState::new();

    // Карта рендерится после того, как и snapshot (DOM-секция), и mermaid
    // source загружены; повторная загрузка перерисовывает карту. Рендер в
    // mermaid.js асинхронный (включая загрузку ESM-модуля), поэтому auto-fit
    // вызывается по событию proteus-mermaid-rendered из index.html.
    Effect::new(move |_| {
        let code = mermaid.get();
        if code.trim().is_empty() || snapshot.with(Option::is_none) {
            return;
        }
        let _ = render_mermaid_map(&code);
    });

    install_mermaid_rendered_fit(map_view);

    view! {
        <section class="configs-page architecture-page">
            <div class="resume-toolbar">
                <div>
                    <h2>"Architecture"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <div class="toolbar-actions">
                    <button type="button" class="secondary" on:click=copy_mermaid>"Mermaid"</button>
                    <button type="button" class="secondary" on:click=refresh>"Обновить"</button>
                </div>
            </div>
            {move || {
                snapshot
                    .get()
                    .map(|snapshot| view! { <TopologySnapshotView snapshot mermaid=mermaid.get() map=map_view /> }.into_any())
                    .unwrap_or_else(|| {
                        view! {
                            <div class="empty-state">
                                <div class="empty-state-title">"Topology недоступен"</div>
                            </div>
                        }
                        .into_any()
                    })
            }}
        </section>
    }
}

fn load_topology_snapshot(
    set_snapshot: WriteSignal<Option<TopologySnapshot>>,
    set_mermaid: WriteSignal<String>,
    set_status: WriteSignal<String>,
) {
    spawn_local(async move {
        match get_json::<TopologySnapshot>("/inspect/topology").await {
            Ok(snapshot) => {
                let slot_count = snapshot.slots.len();
                let tool_count = snapshot.tools.iter().filter(|tool| tool.registered).count();
                let plugin_count = snapshot.plugins.len();
                let warning_count = snapshot.warnings.len();
                set_snapshot.set(Some(snapshot));
                set_status.set(format!(
                    "{slot_count} slots · {tool_count} tools · {plugin_count} plugins · {warning_count} warnings"
                ));
                match get_text("/inspect/topology.mmd").await {
                    Ok(mermaid) => set_mermaid.set(mermaid),
                    Err(error) => {
                        set_mermaid.set(String::new());
                        set_status.set(format!("Mermaid недоступен: {error}"));
                    }
                }
            }
            Err(error) => {
                set_snapshot.set(None);
                set_mermaid.set(String::new());
                set_status.set(format!("не удалось загрузить topology: {error}"));
            }
        }
    });
}

// ---------- View ----------

#[component]
fn TopologySnapshotView(
    snapshot: TopologySnapshot,
    mermaid: String,
    map: MapViewState,
) -> impl IntoView {
    let slots = slot_views(&snapshot);
    let steps = pipeline_steps(&snapshot, &slots);
    let last_step_index = steps.len().saturating_sub(1);
    let backends = backend_views(&snapshot, &slots);
    let slot_cards = slots
        .iter()
        .filter(|view| view.category != "registry")
        .cloned()
        .collect::<Vec<_>>();

    let model_label = snapshot
        .model
        .as_ref()
        .map(|model| format!("{}/{}", model.provider, model.name))
        .unwrap_or_else(|| "model не выбран".to_owned());
    let config_label = snapshot
        .config_path
        .as_deref()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "(default discovery / none)".to_owned());
    let registered_tool_count = snapshot.tools.iter().filter(|tool| tool.registered).count();
    let provided_only_count = snapshot.tools.len() - registered_tool_count;
    let loaded_plugin_count = snapshot
        .plugins
        .iter()
        .filter(|plugin| plugin.status == "loaded")
        .count();

    let plugins = snapshot.plugins.clone();
    let plugin_chips = plugins
        .iter()
        .map(|plugin| {
            (
                format!("{}:{}", plugin.name, plugin.path),
                plugin.clone(),
                plugin_contributions(&snapshot, plugin),
            )
        })
        .collect::<Vec<_>>();
    let tools = snapshot.tools.clone();
    let warnings = snapshot.warnings.clone();
    let mermaid_preview = mermaid.clone();
    let mermaid_copy_text = mermaid.clone();
    let copy_mermaid_preview = move |event: MouseEvent| {
        event.stop_propagation();
        if !mermaid_copy_text.trim().is_empty() {
            copy_to_clipboard(mermaid_copy_text.clone());
        }
    };
    let (tool_filter, set_tool_filter) = signal("all".to_owned());
    let tools_for_filter = tools.clone();
    let filtered_tools = move || {
        let filter = tool_filter.get();
        tools_for_filter
            .clone()
            .into_iter()
            .filter(|tool| match filter.as_str() {
                "enabled" => tool.enabled && tool.registered,
                "disabled" => !tool.enabled || !tool.registered,
                "read" => tool.safety == "ReadOnly",
                "write" => tool.safety != "ReadOnly",
                "plugin" => tool.provider_plugin.is_some(),
                _ => true,
            })
            .collect::<Vec<_>>()
    };

    view! {
        <div class="configs-scroll architecture-scroll">
            <section class="config-overview">
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"runtime"</span>
                        <strong>{non_empty(&snapshot.profile, "default")}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"cwd"</span>
                        <code>{non_empty(&snapshot.cwd, "-")}</code>
                    </div>
                    <div class="config-kv">
                        <span>"config"</span>
                        <code>{config_label}</code>
                    </div>
                    <div class="config-kv">
                        <span>"mode / epoch"</span>
                        <code>{format!("{} / {}", non_empty(&snapshot.permission_mode, "-"), snapshot.module_epoch)}</code>
                    </div>
                </article>
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"model"</span>
                        <strong>{model_label}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"plugins"</span>
                        <code>{format!("{loaded_plugin_count}/{} loaded", snapshot.plugins.len())}</code>
                    </div>
                    <div class="config-kv">
                        <span>"warnings"</span>
                        <code>{snapshot.warnings.len().to_string()}</code>
                    </div>
                </article>
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"tools"</span>
                        <strong>{format!("{registered_tool_count} registered")}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"provided, не в registry"</span>
                        <code>{provided_only_count.to_string()}</code>
                    </div>
                    <div class="config-kv">
                        <span>"config files"</span>
                        <code>{snapshot.config_files.len().to_string()}</code>
                    </div>
                </article>
            </section>

            <TopologyMapView map />

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Turn pipeline"</h3>
                    <span>"путь одного запроса"</span>
                </div>
                <div class="pipeline-flow">
                    {steps
                        .into_iter()
                        .enumerate()
                        .map(|(index, step)| {
                            let card_class = if step.missing {
                                "pipeline-step missing"
                            } else if step.id == "workflow" || step.id == "config" {
                                "pipeline-step anchor"
                            } else {
                                "pipeline-step"
                            };
                            view! {
                                <>
                                    <article class=card_class>
                                        <span class="panel-kicker">{step.label}</span>
                                        <code>{step.detail}</code>
                                        <span class="pipeline-source">{step.source}</span>
                                    </article>
                                    {if index < last_step_index {
                                        view! { <span class="pipeline-arrow" aria-hidden="true">"→"</span> }.into_any()
                                    } else {
                                        view! { <></> }.into_any()
                                    }}
                                </>
                            }
                        })
                        .collect_view()}
                </div>
                <div class="backend-row">
                    <For
                        each=move || backends.clone()
                        key=|backend| backend.slot_id.clone()
                        children=move |backend| {
                            let card_class = if backend.missing {
                                "backend-card missing"
                            } else {
                                "backend-card"
                            };
                            let used_by = backend.used_by.clone();
                            view! {
                                <article class=card_class>
                                    <div class="backend-card-head">
                                        <span class="panel-kicker">{backend.slot_id}</span>
                                        <span class="topology-muted">{backend.role}</span>
                                    </div>
                                    <code>{backend.active_label}</code>
                                    <span class="pipeline-source">{backend.source}</span>
                                    {if used_by.is_empty() {
                                        view! { <></> }.into_any()
                                    } else {
                                        view! {
                                            <div class="config-chip-row">
                                                <For
                                                    each=move || used_by.clone()
                                                    key=|tool| tool.clone()
                                                    children=move |tool| {
                                                        view! { <span class="config-chip">{format!("← {tool}")}</span> }
                                                    }
                                                />
                                            </div>
                                        }.into_any()
                                    }}
                                </article>
                            }
                        }
                    />
                </div>
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Slots"</h3>
                    <span>{slot_cards.len()}</span>
                </div>
                <div class="topology-node-grid">
                    <For
                        each=move || slot_cards.clone()
                        key=|view| view.slot.id.clone()
                        children=move |view| {
                            let slot_id = view.slot.id.clone();
                            let title = non_empty(&view.slot.title, &slot_id);
                            let active_label = view
                                .slot
                                .active_module
                                .clone()
                                .unwrap_or_else(|| "module не выбран".to_owned());
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
                            let card_class = if is_active {
                                "topology-node-card active"
                            } else if view.slot.required {
                                "topology-node-card missing"
                            } else {
                                "topology-node-card"
                            };
                            let status_class = if is_active {
                                "status-badge completed"
                            } else if view.slot.required {
                                "status-badge failed"
                            } else {
                                "status-badge disconnected"
                            };
                            let status_label = if is_active {
                                "active"
                            } else if view.slot.required {
                                "missing"
                            } else {
                                "off"
                            };
                            let required_label = if view.slot.required { "required" } else { "optional" };
                            let alternatives = view.alternatives.clone();
                            view! {
                                <article class=card_class>
                                    <div class="topology-node-head">
                                        <div>
                                            <span class="panel-kicker">{slot_id}</span>
                                            <strong>{title}</strong>
                                        </div>
                                        <div class="tool-badges">
                                            <span class=status_class>{status_label}</span>
                                            <span class="status-badge idle">{required_label}</span>
                                        </div>
                                    </div>
                                    <div class="topology-node-body">
                                        <code>{active_label}</code>
                                        <span>{source}</span>
                                        <p>{description}</p>
                                    </div>
                                    {if alternatives.is_empty() {
                                        view! { <></> }.into_any()
                                    } else {
                                        view! {
                                            <div class="topology-available-inline">
                                                <span>"available"</span>
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
                                            </div>
                                        }.into_any()
                                    }}
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
                {if plugin_chips.is_empty() {
                    view! { <div class="config-empty">"(plugins не найдены)"</div> }.into_any()
                } else {
                    view! {
                        <div class="topology-node-grid plugins">
                            <For
                                each=move || plugin_chips.clone()
                                key=|(key, _, _)| key.clone()
                                children=move |(_, plugin, contributions)| {
                                    let status = non_empty(&plugin.status, "unknown");
                                    let badge_class = plugin_badge_class(&status);
                                    let version = non_empty(&plugin.version, "-");
                                    let description = plugin
                                        .description
                                        .clone()
                                        .unwrap_or_else(|| plugin.path.clone());
                                    view! {
                                        <article class="topology-node-card plugin">
                                            <div class="topology-node-head">
                                                <div>
                                                    <span class="panel-kicker">"plugin"</span>
                                                    <strong>{plugin.name.clone()}</strong>
                                                </div>
                                                <span class=badge_class>
                                                    <span class="dot"></span>
                                                    {status}
                                                </span>
                                            </div>
                                            <div class="topology-node-body">
                                                <code>{version}</code>
                                                <p>{description}</p>
                                            </div>
                                            {if contributions.is_empty() {
                                                view! { <div class="topology-muted">"contributions не зарегистрированы"</div> }.into_any()
                                            } else {
                                                view! {
                                                    <div class="topology-edge-row">
                                                        <For
                                                            each=move || contributions.clone()
                                                            key=|chip| chip.key.clone()
                                                            children=move |chip| {
                                                                view! { <span class=chip.state.chip_class()>{chip.text}</span> }
                                                            }
                                                        />
                                                    </div>
                                                }.into_any()
                                            }}
                                        </article>
                                    }
                                }
                            />
                        </div>
                    }.into_any()
                }}
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Tools"</h3>
                    <span>{tools.len()}</span>
                </div>
                <div class="topology-filter-row">
                    <button type="button" class:active=move || tool_filter.get() == "all" on:click=move |_| set_tool_filter.set("all".to_owned())>"all"</button>
                    <button type="button" class:active=move || tool_filter.get() == "enabled" on:click=move |_| set_tool_filter.set("enabled".to_owned())>"enabled"</button>
                    <button type="button" class:active=move || tool_filter.get() == "disabled" on:click=move |_| set_tool_filter.set("disabled".to_owned())>"disabled"</button>
                    <button type="button" class:active=move || tool_filter.get() == "read" on:click=move |_| set_tool_filter.set("read".to_owned())>"read"</button>
                    <button type="button" class:active=move || tool_filter.get() == "write" on:click=move |_| set_tool_filter.set("write".to_owned())>"write"</button>
                    <button type="button" class:active=move || tool_filter.get() == "plugin" on:click=move |_| set_tool_filter.set("plugin".to_owned())>"plugin"</button>
                </div>
                <div class="config-list">
                    <For
                        each=filtered_tools
                        key=|tool| format!("{}:{}", tool.name, tool.source)
                        children=move |tool| {
                            let registration_class = if tool.registered {
                                "status-badge completed"
                            } else {
                                "status-badge disconnected"
                            };
                            let registration_label = if tool.registered { "registered" } else { "provided" };
                            let enabled_class = if tool.enabled {
                                "status-badge completed"
                            } else {
                                "status-badge failed"
                            };
                            let enabled_label = if tool.enabled { "enabled" } else { "disabled" };
                            let source = tool
                                .provider_plugin
                                .clone()
                                .map(|plugin| format!("plugin:{plugin}"))
                                .unwrap_or_else(|| tool.source.clone());
                            view! {
                                <article class="config-list-item topology-tool-item">
                                    <div class="config-list-main">
                                        <div class="config-list-title">
                                            <strong>{tool.name.clone()}</strong>
                                            <code>{source}</code>
                                        </div>
                                        <p>{tool.description.clone()}</p>
                                        <details class="topology-schema">
                                            <summary>"schema"</summary>
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

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Warnings"</h3>
                    <span>{warnings.len()}</span>
                </div>
                {if warnings.is_empty() {
                    view! { <div class="config-empty">"(none)"</div> }.into_any()
                } else {
                    view! {
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
                    }.into_any()
                }}
            </section>

            <details class="config-section mermaid-details">
                <summary class="config-section-header">
                    <h3>"Mermaid"</h3>
                    <div class="mermaid-summary-actions">
                        <span>{format!("{} bytes", mermaid_preview.len())}</span>
                        <button
                            type="button"
                            class="secondary mermaid-copy-button"
                            on:click=copy_mermaid_preview
                        >
                            "copy"
                        </button>
                    </div>
                </summary>
                <pre class="mermaid-preview">{mermaid_preview}</pre>
            </details>
        </div>
    }
}

fn plugin_badge_class(status: &str) -> &'static str {
    if status.starts_with("error") || status == "failed" {
        "status-badge failed"
    } else if status == "loaded" {
        "status-badge completed"
    } else {
        "status-badge disconnected"
    }
}

fn schema_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| compact_json(value))
}
