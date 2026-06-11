use std::collections::{BTreeSet, HashMap};

use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;
use web_sys::{MouseEvent, window};

use crate::api::{get_json, get_text, post_json};
use crate::markdown::markdown_html;
use crate::types::*;
use crate::ui_utils::{compact_json, copy_to_clipboard, short_id, short_path};

const RUNTIME_SLOT_ORDER: [&str; 11] = [
    "model",
    "workflow",
    "context",
    "tool_exposure",
    "policy",
    "search",
    "patch",
    "memory",
    "memory_policy",
    "compactor",
    "renderer",
];

#[component]
pub(crate) fn ResumeView() -> impl IntoView {
    let (sessions, set_sessions) = signal(Vec::<SessionSummary>::new());
    let (status, set_status) = signal("загружаю сессии".to_owned());

    load_sessions(set_sessions, set_status);

    let refresh = move |_| load_sessions(set_sessions, set_status);
    let resume = move |session_dir: String| {
        set_status.set("возвращаю сессию".to_owned());
        spawn_local(async move {
            match post_json(
                "/resume",
                &ResumeSessionRequest {
                    id: Some("resume".to_owned()),
                    session_dir,
                },
            )
            .await
            {
                Ok(StdioOutput::Response { ok: true, .. }) => {
                    set_status.set("сессия открыта".to_owned());
                    if let Some(window) = window() {
                        let _ = window.location().set_href("/");
                    }
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    set_status.set(error.unwrap_or_else(|| "не удалось открыть сессию".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    set_status.set("неожиданное событие resume".to_owned());
                }
                Err(error) => set_status.set(format!("не удалось открыть сессию: {error}")),
            }
        });
    };

    view! {
        <section class="resume-page">
            <div class="resume-toolbar">
                <div>
                    <h2>"Прошлые сессии"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <button type="button" class="secondary" on:click=refresh>"Обновить"</button>
            </div>
            {move || {
                let items = sessions.get();
                if items.is_empty() {
                    view! {
                        <div class="empty-state">
                            <div class="empty-state-title">"Сохранённых сессий нет"</div>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="resume-list">
                            <For
                                each=move || sessions.get()
                                key=|session| session.session_dir.clone()
                                children=move |session| {
                                    let session_dir = session.session_dir.clone();
                                    let workspace = session.workspace_path.clone().unwrap_or_else(|| "неизвестный workspace".to_owned());
                                    let session_id = session
                                        .session_id
                                        .as_deref()
                                        .map(short_id)
                                        .unwrap_or("legacy")
                                        .to_owned();
                                    view! {
                                        <article class="resume-item">
                                            <div class="resume-item-main">
                                                <div class="resume-item-header">
                                                    <strong>{short_path(&workspace)}</strong>
                                                    <code>{session_id}</code>
                                                </div>
                                                <p>{session.preview.clone().unwrap_or_else(|| "Нет превью диалога".to_owned())}</p>
                                                <div class="resume-meta">
                                                    <span>{workspace}</span>
                                                    <span>{format!("{} сообщений", session.message_count)}</span>
                                                </div>
                                            </div>
                                            <button
                                                type="button"
                                                class="btn-primary"
                                                disabled=!session.resumable
                                                on:click=move |_| resume(session_dir.clone())
                                            >
                                                "Открыть"
                                            </button>
                                        </article>
                                    }
                                }
                            />
                        </div>
                    }.into_any()
                }
            }}
        </section>
    }
}

fn load_sessions(set_sessions: WriteSignal<Vec<SessionSummary>>, set_status: WriteSignal<String>) {
    spawn_local(async move {
        match get_json::<Vec<SessionSummary>>("/sessions").await {
            Ok(items) => {
                let count = items.len();
                set_sessions.set(items);
                set_status.set(format!("{count} сессий"));
            }
            Err(error) => set_status.set(format!("не удалось загрузить сессии: {error}")),
        }
    });
}

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
                    .map(|snapshot| view! { <TopologySnapshotView snapshot mermaid=mermaid.get() /> }.into_any())
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
        let snapshot_result = get_json::<TopologySnapshot>("/inspect/topology").await;
        match snapshot_result {
            Ok(snapshot) => {
                let slot_count = snapshot.slots.len();
                let tool_count = snapshot.tools.iter().filter(|tool| tool.registered).count();
                let plugin_count = snapshot.plugins.len();
                let edge_count = snapshot.edges.len();
                set_snapshot.set(Some(snapshot));
                set_status.set(format!(
                    "{slot_count} slots · {tool_count} tools · {plugin_count} plugins · {edge_count} edges"
                ));
                match get_text("/inspect/topology.runtime.mmd").await {
                    Ok(mermaid) => set_mermaid.set(mermaid),
                    Err(error) => {
                        set_mermaid.set(String::new());
                        set_status.set(format!(
                            "{slot_count} slots · {tool_count} tools · {plugin_count} plugins · {edge_count} edges · Mermaid недоступен: {error}"
                        ));
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

#[component]
fn TopologySnapshotView(snapshot: TopologySnapshot, mermaid: String) -> impl IntoView {
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
    let disabled_plugin_tool_count = snapshot
        .tools
        .iter()
        .filter(|tool| !tool.registered && tool.provider_plugin.is_some())
        .count();
    let loaded_plugin_count = snapshot
        .plugins
        .iter()
        .filter(|plugin| plugin.status == "loaded")
        .count();
    let topology_snapshot = snapshot.clone();
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
                "enabled" => tool.enabled,
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
                        <strong>{non_empty(snapshot.profile.as_str(), "default")}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"cwd"</span>
                        <code>{non_empty(snapshot.cwd.as_str(), "-")}</code>
                    </div>
                    <div class="config-kv">
                        <span>"config"</span>
                        <code>{config_label}</code>
                    </div>
                    <div class="config-kv">
                        <span>"epoch"</span>
                        <code>{snapshot.module_epoch.to_string()}</code>
                    </div>
                </article>
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"model"</span>
                        <strong>{model_label}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"mode"</span>
                        <code>{non_empty(snapshot.permission_mode.as_str(), "-")}</code>
                    </div>
                    <div class="config-kv">
                        <span>"plugins"</span>
                        <code>{format!("{loaded_plugin_count}/{}", snapshot.plugins.len())}</code>
                    </div>
                    <div class="config-kv">
                        <span>"warnings"</span>
                        <code>{snapshot.warnings.len().to_string()}</code>
                    </div>
                </article>
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"tools"</span>
                        <strong>{registered_tool_count.to_string()}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"provided"</span>
                        <code>{snapshot.tools.len().to_string()}</code>
                    </div>
                    <div class="config-kv">
                        <span>"disabled"</span>
                        <code>{disabled_plugin_tool_count.to_string()}</code>
                    </div>
                    <div class="config-kv">
                        <span>"files"</span>
                        <code>{snapshot.config_files.len().to_string()}</code>
                    </div>
                </article>
            </section>

            <TopologyMapView snapshot=topology_snapshot />

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
                            let status_class = if tool.registered {
                                "status-badge completed"
                            } else {
                                "status-badge disconnected"
                            };
                            view! {
                                <article class="config-list-item topology-tool-item">
                                    <div class="config-list-main">
                                        <div class="config-list-title">
                                            <strong>{tool.name.clone()}</strong>
                                            <code>{tool.source.clone()}</code>
                                        </div>
                                        <p>{tool.description.clone()}</p>
                                        <details class="topology-schema">
                                            <summary>"schema"</summary>
                                            <pre>{schema_json(&tool.input_schema)}</pre>
                                        </details>
                                    </div>
                                    <div class="tool-badges">
                                        <span class="status-badge idle">{tool.safety}</span>
                                        <span class=status_class>{if tool.registered { "registered" } else { "provided" }}</span>
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
                                    view! {
                                        <article class="config-list-item">
                                            <div class="config-list-main">
                                                <div class="config-list-title">
                                                    <strong>{warning.severity}</strong>
                                                </div>
                                                <p>{warning.message}</p>
                                            </div>
                                            <span class="status-badge disconnected">"warning"</span>
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

#[derive(Clone, Debug)]
struct TopologySlotNode {
    slot: TopologySlot,
    active_module: Option<TopologyModule>,
    available_modules: Vec<TopologyModule>,
    incoming_edges: Vec<TopologyEdge>,
    outgoing_edges: Vec<TopologyEdge>,
}

#[derive(Clone, Debug)]
struct TopologyPluginNode {
    plugin: TopologyPlugin,
    contribution_edges: Vec<TopologyEdge>,
    module_edges: Vec<TopologyEdge>,
    tool_edges: Vec<TopologyEdge>,
    provider_edges: Vec<TopologyEdge>,
}

#[derive(Clone, Debug)]
struct TopologyToolNode {
    tool: TopologyTool,
    incoming_edges: Vec<TopologyEdge>,
    outgoing_edges: Vec<TopologyEdge>,
}

#[derive(Clone, Debug)]
struct TopologyRuntimeItem {
    label: String,
    detail: String,
    source: String,
    role: &'static str,
    class_name: String,
}

#[derive(Clone, Debug)]
struct TopologyGraphModel {
    width: f32,
    height: f32,
    lanes: Vec<TopologyGraphLane>,
    nodes: Vec<TopologyGraphNode>,
    lines: Vec<TopologyGraphLine>,
}

#[derive(Clone, Debug)]
struct TopologyGraphLane {
    label: &'static str,
    x: f32,
}

#[derive(Clone, Debug)]
struct TopologyGraphNode {
    id: String,
    label: String,
    detail: String,
    badge: String,
    class_name: &'static str,
    x: f32,
    y: f32,
}

#[derive(Clone, Debug)]
struct TopologyGraphLine {
    key: String,
    class_name: &'static str,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

#[derive(Clone, Debug)]
struct DanglingTopologyNode {
    id: String,
    incoming_edges: Vec<TopologyEdge>,
    outgoing_edges: Vec<TopologyEdge>,
}

#[component]
fn TopologyMapView(snapshot: TopologySnapshot) -> impl IntoView {
    let slot_nodes = topology_slot_nodes(&snapshot);
    let plugin_nodes = topology_plugin_nodes(&snapshot);
    let tool_nodes = topology_tool_nodes(&snapshot);
    let mut available_modules = snapshot
        .modules
        .iter()
        .filter(|module| !module.active)
        .cloned()
        .collect::<Vec<_>>();
    available_modules.sort_by(|left, right| {
        left.slot
            .cmp(&right.slot)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut available_tools = snapshot
        .tools
        .iter()
        .filter(|tool| !tool.enabled || !tool.registered)
        .cloned()
        .collect::<Vec<_>>();
    available_tools.sort_by(|left, right| left.name.cmp(&right.name));
    let dangling_nodes = dangling_topology_nodes(&snapshot);
    let registry_edges = snapshot
        .edges
        .iter()
        .filter(|edge| edge.from == "tools" || edge.to == "tools")
        .cloned()
        .collect::<Vec<_>>();
    let registered_tool_nodes = tool_nodes
        .iter()
        .filter(|node| node.tool.registered)
        .cloned()
        .collect::<Vec<_>>();
    let edge_count = snapshot.edges.len();
    let runtime_edge_count = snapshot
        .edges
        .iter()
        .filter(|edge| edge.kind == "runtime")
        .count();
    let provides_edge_count = snapshot
        .edges
        .iter()
        .filter(|edge| edge.kind == "provides")
        .count();
    let uses_edge_count = snapshot
        .edges
        .iter()
        .filter(|edge| edge.kind == "uses")
        .count();
    let active_slot_count = slot_nodes
        .iter()
        .filter(|node| node.active_module.is_some())
        .count();
    let loaded_plugin_count = plugin_nodes
        .iter()
        .filter(|node| node.plugin.status == "loaded")
        .count();
    let has_available =
        !available_modules.is_empty() || !available_tools.is_empty() || !dangling_nodes.is_empty();
    let graph_model = topology_graph_model(&snapshot, &slot_nodes, &plugin_nodes, &tool_nodes);
    let graph_lanes = graph_model.lanes.clone();
    let graph_nodes = graph_model.nodes.clone();
    let graph_lines = graph_model.lines.clone();
    let graph_width = graph_model.width;
    let graph_height = graph_model.height;
    let runtime_items = topology_runtime_items(&snapshot, &slot_nodes);

    view! {
        <>
            <section class="config-section topology-map-section">
                <div class="config-section-header">
                    <h3>"Runtime path"</h3>
                    <span>"active product path"</span>
                </div>
                <div class="topology-tool-grid">
                    <For
                        each=move || runtime_items.clone()
                        key=|item| item.label.clone()
                        children=move |item| {
                            view! {
                                <article class=item.class_name>
                                    <div class="topology-tool-title">
                                        <strong>{item.label}</strong>
                                        <span>{item.role}</span>
                                    </div>
                                    <code>{item.detail}</code>
                                    <span class="topology-muted">{item.source}</span>
                                </article>
                            }
                        }
                    />
                </div>
            </section>

            <section class="config-section topology-map-section">
                <div class="config-section-header">
                    <h3>"Diagnostic graph"</h3>
                    <span>{format!("{edge_count} edges")}</span>
                </div>
                <div class="topology-map-summary">
                    <span>
                        <strong>{active_slot_count}</strong>
                        <small>"active slots"</small>
                    </span>
                    <span>
                        <strong>{loaded_plugin_count}</strong>
                        <small>"loaded plugins"</small>
                    </span>
                    <span>
                        <strong>{registered_tool_nodes.len()}</strong>
                        <small>"registry tools"</small>
                    </span>
                    <span>
                        <strong>{runtime_edge_count}</strong>
                        <small>"runtime"</small>
                    </span>
                    <span>
                        <strong>{provides_edge_count}</strong>
                        <small>"provides"</small>
                    </span>
                    <span>
                        <strong>{uses_edge_count}</strong>
                        <small>"uses"</small>
                    </span>
                </div>
                <div class="topology-graph-scroll">
                    <div
                        class="topology-graph"
                        style=format!("width: {graph_width}px; height: {graph_height}px;")
                    >
                        <svg
                            class="topology-graph-svg"
                            viewBox=format!("0 0 {graph_width} {graph_height}")
                            aria-hidden="true"
                        >
                            <defs>
                                <marker
                                    id="topology-arrow"
                                    markerWidth="8"
                                    markerHeight="8"
                                    refX="7"
                                    refY="4"
                                    orient="auto"
                                >
                                    <path d="M0,0 L8,4 L0,8 Z"></path>
                                </marker>
                            </defs>
                            <For
                                each=move || graph_lines.clone()
                                key=|line| line.key.clone()
                                children=move |line| {
                                    view! {
                                        <line
                                            class=line.class_name
                                            x1=line.x1.to_string()
                                            y1=line.y1.to_string()
                                            x2=line.x2.to_string()
                                            y2=line.y2.to_string()
                                        />
                                    }
                                }
                            />
                        </svg>
                        <For
                            each=move || graph_lanes.clone()
                            key=|lane| lane.label
                            children=move |lane| {
                                view! {
                                    <div
                                        class="topology-graph-lane-label"
                                        style=format!("left: {}px;", lane.x)
                                    >
                                        {lane.label}
                                    </div>
                                }
                            }
                        />
                        <For
                            each=move || graph_nodes.clone()
                            key=|node| node.id.clone()
                            children=move |node| {
                                view! {
                                    <article
                                        class=node.class_name
                                        style=format!("left: {}px; top: {}px;", node.x, node.y)
                                    >
                                        <div>
                                            <strong>{node.label}</strong>
                                            <span>{node.detail}</span>
                                        </div>
                                        <code>{node.badge}</code>
                                    </article>
                                }
                            }
                        />
                    </div>
                </div>
            </section>

            <section class="config-section topology-map-section">
                <div class="config-section-header">
                    <h3>"Active slots"</h3>
                    <span>{format!("{active_slot_count}/{}", slot_nodes.len())}</span>
                </div>
                <div class="topology-node-grid">
                    <For
                        each=move || slot_nodes.clone()
                        key=|node| node.slot.id.clone()
                        children=move |node| {
                            let slot_id = node.slot.id.clone();
                            let title = non_empty(&node.slot.title, &slot_id);
                            let active_label = node
                                .slot
                                .active_module
                                .clone()
                                .unwrap_or_else(|| "module не выбран".to_owned());
                            let active_source = node
                                .active_module
                                .as_ref()
                                .map(|module| module_source_label(&module.source))
                                .unwrap_or_else(|| "-".to_owned());
                            let description = node
                                .active_module
                                .as_ref()
                                .and_then(|module| module.description.clone())
                                .unwrap_or_else(|| node.slot.responsibility.clone());
                            let card_class = if node.active_module.is_some() {
                                "topology-node-card active"
                            } else {
                                "topology-node-card missing"
                            };
                            let status_class = if node.active_module.is_some() {
                                "status-badge completed"
                            } else {
                                "status-badge disconnected"
                            };
                            let status_label = if node.active_module.is_some() { "active" } else { "missing" };
                            let required_class = if node.slot.required {
                                "status-badge idle"
                            } else {
                                "status-badge disconnected"
                            };
                            let required_label = if node.slot.required { "required" } else { "optional" };
                            let incoming_edges = node.incoming_edges.clone();
                            let outgoing_edges = node.outgoing_edges.clone();
                            let available_modules = node.available_modules.clone();
                            view! {
                                <article class=card_class>
                                    <div class="topology-node-head">
                                        <div>
                                            <span class="panel-kicker">{slot_id}</span>
                                            <strong>{title}</strong>
                                        </div>
                                        <div class="tool-badges">
                                            <span class=status_class>{status_label}</span>
                                            <span class=required_class>{required_label}</span>
                                        </div>
                                    </div>
                                    <div class="topology-node-body">
                                        <code>{active_label}</code>
                                        <span>{active_source}</span>
                                        <p>{description}</p>
                                    </div>
                                    {if incoming_edges.is_empty() && outgoing_edges.is_empty() {
                                        view! { <div class="topology-muted">"edge-связей нет"</div> }.into_any()
                                    } else {
                                        view! {
                                            <div class="topology-edge-row">
                                                <For
                                                    each=move || incoming_edges.clone()
                                                    key=topology_edge_key
                                                    children=move |edge| {
                                                        view! { <span class="topology-edge-chip incoming">{incoming_edge_chip_label(&edge)}</span> }
                                                    }
                                                />
                                                <For
                                                    each=move || outgoing_edges.clone()
                                                    key=topology_edge_key
                                                    children=move |edge| {
                                                        view! { <span class="topology-edge-chip outgoing">{outgoing_edge_chip_label(&edge)}</span> }
                                                    }
                                                />
                                            </div>
                                        }.into_any()
                                    }}
                                    {if available_modules.is_empty() {
                                        view! { <></> }.into_any()
                                    } else {
                                        view! {
                                            <div class="topology-available-inline">
                                                <span>"available"</span>
                                                <div class="config-chip-row compact-chips">
                                                    <For
                                                        each=move || available_modules.clone()
                                                        key=|module| format!("{}:{}", module.slot, module.id)
                                                        children=move |module| {
                                                            view! { <span class="config-chip">{module.id}</span> }
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

            <section class="config-section topology-map-section">
                <div class="config-section-header">
                    <h3>"Plugins -> contributions"</h3>
                    <span>{plugin_nodes.len()}</span>
                </div>
                <div class="topology-node-grid plugins">
                    <For
                        each=move || plugin_nodes.clone()
                        key=|node| format!("{}:{}", node.plugin.name, node.plugin.path)
                        children=move |node| {
                            let name = node.plugin.name.clone();
                            let version = non_empty(&node.plugin.version, "-");
                            let status = non_empty(&node.plugin.status, "unknown");
                            let badge_class = topology_plugin_badge_class(&status);
                            let module_count = node.module_edges.len();
                            let tool_count = node.tool_edges.len();
                            let provider_count = node.provider_edges.len();
                            let description = node
                                .plugin
                                .description
                                .clone()
                                .unwrap_or_else(|| format!("{module_count} modules · {tool_count} tools · {provider_count} providers"));
                            let contribution_edges = node.contribution_edges.clone();
                            let fallback_modules = node.plugin.provides.modules.clone();
                            let fallback_tools = node.plugin.provides.tools.clone();
                            let fallback_providers = node.plugin.provides.context_providers.clone();
                            view! {
                                <article class="topology-node-card plugin">
                                    <div class="topology-node-head">
                                        <div>
                                            <span class="panel-kicker">"plugin"</span>
                                            <strong>{name}</strong>
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
                                    {if contribution_edges.is_empty() {
                                        view! {
                                            <div class="topology-edge-row">
                                                <For
                                                    each=move || fallback_modules.clone()
                                                    key=|module| format!("{}:{}", module.slot, module.id)
                                                    children=move |module| {
                                                        view! { <span class="topology-edge-chip available">{format!("module: {}/{}", module.slot, module.id)}</span> }
                                                    }
                                                />
                                                <For
                                                    each=move || fallback_tools.clone()
                                                    key=|tool| tool.name.clone()
                                                    children=move |tool| {
                                                        view! { <span class="topology-edge-chip available">{format!("tool: {}", tool.name)}</span> }
                                                    }
                                                />
                                                <For
                                                    each=move || fallback_providers.clone()
                                                    key=|provider| provider.clone()
                                                    children=move |provider| {
                                                        view! { <span class="topology-edge-chip available">{format!("context: {provider}")}</span> }
                                                    }
                                                />
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <div class="topology-edge-row">
                                                <For
                                                    each=move || contribution_edges.clone()
                                                    key=topology_edge_key
                                                    children=move |edge| {
                                                        view! { <span class="topology-edge-chip outgoing">{outgoing_edge_chip_label(&edge)}</span> }
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

            <section class="config-section topology-map-section">
                <div class="config-section-header">
                    <h3>"Tool registry"</h3>
                    <span>{registered_tool_nodes.len()}</span>
                </div>
                <div class="topology-registry">
                    <article class="topology-registry-card">
                        <div class="topology-node-head">
                            <div>
                                <span class="panel-kicker">"registry"</span>
                                <strong>"ToolRegistry"</strong>
                            </div>
                            <span class="status-badge completed">{format!("{} registered", registered_tool_nodes.len())}</span>
                        </div>
                        <div class="topology-edge-row">
                            <For
                                each=move || registry_edges.clone()
                                key=topology_edge_key
                                children=move |edge| {
                                    let label = if edge.to == "tools" {
                                        incoming_edge_chip_label(&edge)
                                    } else {
                                        outgoing_edge_chip_label(&edge)
                                    };
                                    view! { <span class="topology-edge-chip runtime">{label}</span> }
                                }
                            />
                        </div>
                    </article>
                    <div class="topology-tool-grid">
                        <For
                            each=move || registered_tool_nodes.clone()
                            key=|node| format!("{}:{}", node.tool.name, node.tool.source)
                            children=move |node| {
                                let tool = node.tool.clone();
                                let provider = tool
                                    .provider_plugin
                                    .clone()
                                    .map(|plugin| format!("plugin:{plugin}"))
                                    .unwrap_or_else(|| tool.source.clone());
                                let outgoing_edges = node.outgoing_edges.clone();
                                let incoming_edges = node.incoming_edges.clone();
                                let card_class = if tool.enabled {
                                    "topology-tool-node enabled"
                                } else {
                                    "topology-tool-node disabled"
                                };
                                view! {
                                    <article class=card_class>
                                        <div class="topology-tool-title">
                                            <strong>{tool.name.clone()}</strong>
                                            <span>{tool.safety.clone()}</span>
                                        </div>
                                        <code>{provider}</code>
                                        {if incoming_edges.is_empty() && outgoing_edges.is_empty() {
                                            view! { <span class="topology-muted">"registry only"</span> }.into_any()
                                        } else {
                                            view! {
                                                <div class="topology-edge-row tight">
                                                    <For
                                                        each=move || incoming_edges.clone()
                                                        key=topology_edge_key
                                                        children=move |edge| {
                                                            view! { <span class="topology-edge-chip incoming">{incoming_edge_chip_label(&edge)}</span> }
                                                        }
                                                    />
                                                    <For
                                                        each=move || outgoing_edges.clone()
                                                        key=topology_edge_key
                                                        children=move |edge| {
                                                            view! { <span class="topology-edge-chip outgoing">{outgoing_edge_chip_label(&edge)}</span> }
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
                </div>
            </section>

            {if has_available {
                view! {
                    <section class="config-section topology-map-section">
                        <div class="config-section-header">
                            <h3>"Diagnostic dangling / available"</h3>
                            <span>{format!("{} items", available_modules.len() + available_tools.len() + dangling_nodes.len())}</span>
                        </div>
                        <div class="topology-available-grid">
                            {if available_modules.is_empty() {
                                view! { <></> }.into_any()
                            } else {
                                view! {
                                    <div class="topology-available-group">
                                        <h4>"available modules"</h4>
                                        <div class="topology-edge-row">
                                            <For
                                                each=move || available_modules.clone()
                                                key=|module| format!("{}:{}", module.slot, module.id)
                                                children=move |module| {
                                                    let source = module_source_label(&module.source);
                                                    view! {
                                                        <span class="topology-edge-chip available">
                                                            {format!("{}/{} · {source}", module.slot, module.id)}
                                                        </span>
                                                    }
                                                }
                                            />
                                        </div>
                                    </div>
                                }.into_any()
                            }}
                            {if available_tools.is_empty() {
                                view! { <></> }.into_any()
                            } else {
                                view! {
                                    <div class="topology-available-group">
                                        <h4>"available tools"</h4>
                                        <div class="topology-edge-row">
                                            <For
                                                each=move || available_tools.clone()
                                                key=|tool| format!("{}:{}", tool.name, tool.source)
                                                children=move |tool| {
                                                    let state = if tool.registered { "disabled" } else { "provided" };
                                                    view! {
                                                        <span class="topology-edge-chip available">
                                                            {format!("{} · {state}", tool.name)}
                                                        </span>
                                                    }
                                                }
                                            />
                                        </div>
                                    </div>
                                }.into_any()
                            }}
                            {if dangling_nodes.is_empty() {
                                view! { <></> }.into_any()
                            } else {
                                view! {
                                    <div class="topology-available-group">
                                        <h4>"dangling edge nodes"</h4>
                                        <div class="topology-edge-row">
                                            <For
                                                each=move || dangling_nodes.clone()
                                                key=|node| node.id.clone()
                                                children=move |node| {
                                                    view! {
                                                        <span class="topology-edge-chip dangling">
                                                            {format!(
                                                                "{} · in:{} out:{}",
                                                                topology_node_label(&node.id),
                                                                node.incoming_edges.len(),
                                                                node.outgoing_edges.len()
                                                            )}
                                                        </span>
                                                    }
                                                }
                                            />
                                        </div>
                                    </div>
                                }.into_any()
                            }}
                        </div>
                    </section>
                }.into_any()
            } else {
                view! { <></> }.into_any()
            }}
        </>
    }
}

fn module_source_label(source: &TopologyModuleSource) -> String {
    match source.kind.as_str() {
        "plugin" => source
            .name
            .as_deref()
            .map(|name| format!("plugin:{name}"))
            .unwrap_or_else(|| "plugin".to_owned()),
        "builtin" => "builtin".to_owned(),
        "config" => "config".to_owned(),
        _ => "unknown".to_owned(),
    }
}

fn topology_runtime_items(
    snapshot: &TopologySnapshot,
    slot_nodes: &[TopologySlotNode],
) -> Vec<TopologyRuntimeItem> {
    let mut items = [
        ("workflow", "turn loop"),
        ("context", "context build"),
        ("tool_exposure", "tool selection"),
        ("model", "model call"),
        ("policy", "approval gate"),
    ]
    .into_iter()
    .filter_map(|(slot_id, role)| topology_runtime_slot_item(slot_nodes, slot_id, role))
    .collect::<Vec<_>>();

    let registered = snapshot.tools.iter().filter(|tool| tool.registered).count();
    let enabled = snapshot.tools.iter().filter(|tool| tool.enabled).count();
    items.push(TopologyRuntimeItem {
        label: "tools".to_owned(),
        detail: format!("{registered} registered · {enabled} enabled"),
        source: "ToolRegistry".to_owned(),
        role: "capabilities",
        class_name: if registered == 0 {
            "topology-tool-node disabled".to_owned()
        } else {
            "topology-tool-node enabled".to_owned()
        },
    });

    items.extend(
        [("patch", "edit backend"), ("search", "repo search"), ("renderer", "final output")]
            .into_iter()
            .filter_map(|(slot_id, role)| topology_runtime_slot_item(slot_nodes, slot_id, role)),
    );
    items
}

fn topology_runtime_slot_item(
    slot_nodes: &[TopologySlotNode],
    slot_id: &str,
    role: &'static str,
) -> Option<TopologyRuntimeItem> {
    let node = slot_nodes.iter().find(|node| node.slot.id == slot_id)?;
    let detail = node
        .slot
        .active_module
        .clone()
        .unwrap_or_else(|| "module не выбран".to_owned());
    let source = node
        .active_module
        .as_ref()
        .map(|module| module_source_label(&module.source))
        .unwrap_or_else(|| "missing".to_owned());
    let class_name = if node.slot.active_module.is_some() {
        "topology-tool-node enabled"
    } else {
        "topology-tool-node disabled"
    };
    Some(TopologyRuntimeItem {
        label: slot_id.to_owned(),
        detail,
        source,
        role,
        class_name: class_name.to_owned(),
    })
}

const TOPOLOGY_GRAPH_WIDTH: f32 = 1120.0;
const TOPOLOGY_GRAPH_NODE_WIDTH: f32 = 190.0;
const TOPOLOGY_GRAPH_NODE_HEIGHT: f32 = 48.0;
const TOPOLOGY_GRAPH_HEIGHT: f32 = 560.0;
const TOPOLOGY_GRAPH_CONFIG_X: f32 = 32.0;
const TOPOLOGY_GRAPH_LEFT_X: f32 = 270.0;
const TOPOLOGY_GRAPH_RUNTIME_X: f32 = 520.0;
const TOPOLOGY_GRAPH_TARGET_X: f32 = 820.0;

fn topology_graph_model(
    snapshot: &TopologySnapshot,
    slot_nodes: &[TopologySlotNode],
    plugin_nodes: &[TopologyPluginNode],
    tool_nodes: &[TopologyToolNode],
) -> TopologyGraphModel {
    let loaded_plugin_count = plugin_nodes
        .iter()
        .filter(|node| node.plugin.status == "loaded")
        .count();
    let provided_module_count = plugin_nodes
        .iter()
        .map(|node| node.plugin.provides.modules.len())
        .sum::<usize>();
    let provided_tool_count = tool_nodes.len();
    let registered_tool_count = tool_nodes
        .iter()
        .filter(|node| node.tool.registered)
        .count();
    let enabled_tool_count = tool_nodes.iter().filter(|node| node.tool.enabled).count();
    let disabled_tool_count = provided_tool_count.saturating_sub(registered_tool_count);
    let support_slots = ["search", "patch", "memory", "memory_policy", "compactor"];
    let support_detail = support_slots
        .iter()
        .filter_map(|id| {
            slot_nodes
                .iter()
                .find(|node| node.slot.id == *id)
                .map(|node| {
                    format!(
                        "{id}:{}",
                        node.slot.active_module.as_deref().unwrap_or("missing")
                    )
                })
        })
        .collect::<Vec<_>>()
        .join(" · ");

    let mut nodes = vec![TopologyGraphNode {
        id: "config".to_owned(),
        label: "config".to_owned(),
        detail: non_empty(snapshot.profile.as_str(), "default"),
        badge: snapshot.permission_mode.clone(),
        class_name: "topology-graph-node config",
        x: TOPOLOGY_GRAPH_CONFIG_X,
        y: 72.0,
    }];

    nodes.push(TopologyGraphNode {
        id: "plugins".to_owned(),
        label: "plugins".to_owned(),
        detail: format!(
            "{loaded_plugin_count}/{} loaded · {provided_module_count} modules",
            plugin_nodes.len()
        ),
        badge: format!("{provided_tool_count} tools"),
        class_name: "topology-graph-node plugin loaded",
        x: TOPOLOGY_GRAPH_CONFIG_X,
        y: 208.0,
    });

    nodes.push(TopologyGraphNode {
        id: "slot:workflow".to_owned(),
        label: "workflow".to_owned(),
        detail: active_slot_module_label(slot_nodes, "workflow"),
        badge: "runtime".to_owned(),
        class_name: "topology-graph-node slot active workflow",
        x: TOPOLOGY_GRAPH_LEFT_X,
        y: 248.0,
    });

    nodes.push(TopologyGraphNode {
        id: "support-slots".to_owned(),
        label: "support slots".to_owned(),
        detail: non_empty(&support_detail, "none"),
        badge: "configured".to_owned(),
        class_name: "topology-graph-node support",
        x: TOPOLOGY_GRAPH_LEFT_X,
        y: 420.0,
    });

    let runtime_slots = [
        ("context", 56.0),
        ("model", 136.0),
        ("tool_exposure", 216.0),
        ("policy", 296.0),
        ("renderer", 376.0),
    ];
    for (slot_id, y) in runtime_slots {
        let available_count = slot_nodes
            .iter()
            .find(|node| node.slot.id == slot_id)
            .map(|node| node.available_modules.len())
            .unwrap_or_default();
        let badge = if slot_id == "tool_exposure" {
            "tool visibility".to_owned()
        } else if available_count == 0 {
            "active".to_owned()
        } else {
            format!("+{available_count} options")
        };
        let class_name = if slot_nodes
            .iter()
            .any(|node| node.slot.id == slot_id && node.active_module.is_some())
        {
            "topology-graph-node slot active"
        } else {
            "topology-graph-node slot missing"
        };
        nodes.push(TopologyGraphNode {
            id: format!("slot:{slot_id}"),
            label: slot_id.to_owned(),
            detail: active_slot_module_label(slot_nodes, slot_id),
            badge,
            class_name,
            x: TOPOLOGY_GRAPH_RUNTIME_X,
            y,
        });
    }

    nodes.push(TopologyGraphNode {
        id: "tools".to_owned(),
        label: "ToolRegistry".to_owned(),
        detail: format!("{registered_tool_count} registered"),
        badge: format!("{enabled_tool_count} enabled"),
        class_name: "topology-graph-node registry",
        x: TOPOLOGY_GRAPH_TARGET_X,
        y: 248.0,
    });

    nodes.push(TopologyGraphNode {
        id: "plugin-tools".to_owned(),
        label: "plugin tools".to_owned(),
        detail: format!("{provided_tool_count} provided · {disabled_tool_count} disabled"),
        badge: if registered_tool_count == 0 {
            "not in registry".to_owned()
        } else {
            "partial".to_owned()
        },
        class_name: if registered_tool_count == provided_tool_count && provided_tool_count > 0 {
            "topology-graph-node tool registered"
        } else {
            "topology-graph-node tool disabled"
        },
        x: TOPOLOGY_GRAPH_TARGET_X,
        y: 344.0,
    });

    let node_positions = nodes
        .iter()
        .map(|node| (node.id.clone(), (node.x, node.y)))
        .collect::<HashMap<_, _>>();
    let mut lines = Vec::new();
    let mut seen_lines = BTreeSet::new();

    push_topology_graph_line(
        &node_positions,
        &mut lines,
        &mut seen_lines,
        "plugins",
        "plugin-tools",
        "provides",
    );
    push_topology_graph_line(
        &node_positions,
        &mut lines,
        &mut seen_lines,
        "plugin-tools",
        "tools",
        if registered_tool_count == 0 {
            "unregistered_tool"
        } else {
            "registered_tool"
        },
    );
    push_topology_graph_line(
        &node_positions,
        &mut lines,
        &mut seen_lines,
        "plugins",
        "support-slots",
        "provides",
    );
    for slot_node in slot_nodes
        .iter()
        .filter(|node| !node.available_modules.is_empty())
    {
        let slot_id = format!("slot:{}", slot_node.slot.id);
        if node_positions.contains_key(&slot_id) {
            push_topology_graph_line(
                &node_positions,
                &mut lines,
                &mut seen_lines,
                "plugins",
                &slot_id,
                "provides",
            );
        }
    }
    push_topology_graph_line(
        &node_positions,
        &mut lines,
        &mut seen_lines,
        "config",
        "support-slots",
        "selects",
    );

    for edge in &snapshot.edges {
        let from = edge.from.clone();
        let to = edge.to.clone();
        if edge.kind != "runtime" && edge.kind != "selects" {
            continue;
        }
        if support_slots.iter().any(|id| from == format!("slot:{id}"))
            || support_slots.iter().any(|id| to == format!("slot:{id}"))
        {
            continue;
        }
        if from == "config" && !node_positions.contains_key(&to) {
            continue;
        }
        push_topology_graph_line(
            &node_positions,
            &mut lines,
            &mut seen_lines,
            &from,
            &to,
            &edge.kind,
        );
    }

    TopologyGraphModel {
        width: TOPOLOGY_GRAPH_WIDTH,
        height: TOPOLOGY_GRAPH_HEIGHT,
        lanes: vec![
            TopologyGraphLane {
                label: "configuration",
                x: TOPOLOGY_GRAPH_CONFIG_X,
            },
            TopologyGraphLane {
                label: "workflow",
                x: TOPOLOGY_GRAPH_LEFT_X,
            },
            TopologyGraphLane {
                label: "runtime slots",
                x: TOPOLOGY_GRAPH_RUNTIME_X,
            },
            TopologyGraphLane {
                label: "tools",
                x: TOPOLOGY_GRAPH_TARGET_X,
            },
        ],
        nodes,
        lines,
    }
}

fn active_slot_module_label(slot_nodes: &[TopologySlotNode], slot_id: &str) -> String {
    slot_nodes
        .iter()
        .find(|node| node.slot.id == slot_id)
        .and_then(|node| node.slot.active_module.clone())
        .unwrap_or_else(|| "module не выбран".to_owned())
}

fn push_topology_graph_line(
    node_positions: &HashMap<String, (f32, f32)>,
    lines: &mut Vec<TopologyGraphLine>,
    seen_lines: &mut BTreeSet<String>,
    from: &str,
    to: &str,
    kind: &str,
) {
    let line_key = format!("{from}>{to}>{kind}");
    if !seen_lines.insert(line_key.clone()) {
        return;
    }
    if let Some(line) =
        topology_graph_line(node_positions, from.to_owned(), to.to_owned(), kind, line_key)
    {
        lines.push(line);
    }
}

fn topology_graph_line(
    node_positions: &HashMap<String, (f32, f32)>,
    from: String,
    to: String,
    kind: &str,
    key: String,
) -> Option<TopologyGraphLine> {
    if from == to {
        return None;
    }
    let (from_x, from_y) = node_positions.get(&from)?;
    let (to_x, to_y) = node_positions.get(&to)?;
    let from_center_y = *from_y + TOPOLOGY_GRAPH_NODE_HEIGHT / 2.0;
    let to_center_y = *to_y + TOPOLOGY_GRAPH_NODE_HEIGHT / 2.0;
    let (x1, x2) = if from_x <= to_x {
        (*from_x + TOPOLOGY_GRAPH_NODE_WIDTH, *to_x)
    } else {
        (*from_x, *to_x + TOPOLOGY_GRAPH_NODE_WIDTH)
    };
    Some(TopologyGraphLine {
        key,
        class_name: topology_graph_line_class(kind),
        x1,
        y1: from_center_y,
        x2,
        y2: to_center_y,
    })
}

fn topology_graph_line_class(kind: &str) -> &'static str {
    match kind {
        "config_selection" | "selects" => "topology-graph-line config",
        "provides" => "topology-graph-line provides",
        "registered_tool" => "topology-graph-line registered",
        "unregistered_tool" => "topology-graph-line disabled",
        "runtime" | "uses" => "topology-graph-line runtime",
        _ => "topology-graph-line",
    }
}

fn topology_slot_nodes(snapshot: &TopologySnapshot) -> Vec<TopologySlotNode> {
    let mut nodes = snapshot
        .slots
        .iter()
        .map(|slot| {
            let slot_node_id = format!("slot:{}", slot.id);
            let active_module = slot.active_module.as_ref().and_then(|active| {
                snapshot
                    .modules
                    .iter()
                    .find(|module| module.slot == slot.id && module.id == *active)
                    .cloned()
            });
            let mut available_modules = snapshot
                .modules
                .iter()
                .filter(|module| module.slot == slot.id && !module.active)
                .cloned()
                .collect::<Vec<_>>();
            available_modules.sort_by(|left, right| left.id.cmp(&right.id));
            TopologySlotNode {
                slot: slot.clone(),
                active_module,
                available_modules,
                incoming_edges: snapshot
                    .edges
                    .iter()
                    .filter(|edge| edge.to == slot_node_id)
                    .cloned()
                    .collect(),
                outgoing_edges: snapshot
                    .edges
                    .iter()
                    .filter(|edge| edge.from == slot_node_id)
                    .cloned()
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        runtime_slot_rank(&left.slot.id)
            .cmp(&runtime_slot_rank(&right.slot.id))
            .then_with(|| left.slot.id.cmp(&right.slot.id))
    });
    nodes
}

fn topology_plugin_nodes(snapshot: &TopologySnapshot) -> Vec<TopologyPluginNode> {
    let mut nodes = snapshot
        .plugins
        .iter()
        .map(|plugin| {
            let plugin_node_id = format!("plugin:{}", plugin.name);
            let contribution_edges = snapshot
                .edges
                .iter()
                .filter(|edge| edge.from == plugin_node_id)
                .cloned()
                .collect::<Vec<_>>();
            let module_edges = contribution_edges
                .iter()
                .filter(|edge| edge.to.starts_with("module:"))
                .cloned()
                .collect::<Vec<_>>();
            let tool_edges = contribution_edges
                .iter()
                .filter(|edge| edge.to.starts_with("tool:"))
                .cloned()
                .collect::<Vec<_>>();
            let provider_edges = contribution_edges
                .iter()
                .filter(|edge| edge.to.starts_with("context_provider:"))
                .cloned()
                .collect::<Vec<_>>();
            TopologyPluginNode {
                plugin: plugin.clone(),
                contribution_edges,
                module_edges,
                tool_edges,
                provider_edges,
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.plugin.name.cmp(&right.plugin.name));
    nodes
}

fn topology_tool_nodes(snapshot: &TopologySnapshot) -> Vec<TopologyToolNode> {
    let mut nodes = snapshot
        .tools
        .iter()
        .map(|tool| {
            let tool_node_id = format!("tool:{}", tool.name);
            TopologyToolNode {
                tool: tool.clone(),
                incoming_edges: snapshot
                    .edges
                    .iter()
                    .filter(|edge| edge.to == tool_node_id)
                    .cloned()
                    .collect(),
                outgoing_edges: snapshot
                    .edges
                    .iter()
                    .filter(|edge| edge.from == tool_node_id)
                    .cloned()
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.tool.name.cmp(&right.tool.name));
    nodes
}

fn dangling_topology_nodes(snapshot: &TopologySnapshot) -> Vec<DanglingTopologyNode> {
    let known_nodes = known_topology_nodes(snapshot);
    let mut dangling_ids = BTreeSet::new();
    for edge in &snapshot.edges {
        if !edge.from.trim().is_empty() && !known_nodes.contains(&edge.from) {
            dangling_ids.insert(edge.from.clone());
        }
        if !edge.to.trim().is_empty() && !known_nodes.contains(&edge.to) {
            dangling_ids.insert(edge.to.clone());
        }
    }

    dangling_ids
        .into_iter()
        .map(|id| DanglingTopologyNode {
            incoming_edges: snapshot
                .edges
                .iter()
                .filter(|edge| edge.to == id)
                .cloned()
                .collect(),
            outgoing_edges: snapshot
                .edges
                .iter()
                .filter(|edge| edge.from == id)
                .cloned()
                .collect(),
            id,
        })
        .collect()
}

fn known_topology_nodes(snapshot: &TopologySnapshot) -> BTreeSet<String> {
    let mut nodes = BTreeSet::new();
    nodes.insert("config".to_owned());
    nodes.insert("tools".to_owned());
    if !snapshot.warnings.is_empty() {
        nodes.insert("warnings".to_owned());
    }
    for slot in &snapshot.slots {
        nodes.insert(format!("slot:{}", slot.id));
    }
    for module in &snapshot.modules {
        nodes.insert(format!("module:{}:{}", module.slot, module.id));
    }
    for plugin in &snapshot.plugins {
        nodes.insert(format!("plugin:{}", plugin.name));
        for provider in &plugin.provides.context_providers {
            nodes.insert(format!("context_provider:{provider}"));
        }
    }
    for tool in &snapshot.tools {
        nodes.insert(format!("tool:{}", tool.name));
    }
    nodes
}

fn runtime_slot_rank(id: &str) -> usize {
    RUNTIME_SLOT_ORDER
        .iter()
        .position(|candidate| *candidate == id)
        .unwrap_or(RUNTIME_SLOT_ORDER.len())
}

fn topology_edge_key(edge: &TopologyEdge) -> String {
    format!(
        "{}>{}>{}>{}",
        edge.from,
        edge.to,
        edge.kind,
        edge.label.as_deref().unwrap_or_default()
    )
}

fn incoming_edge_chip_label(edge: &TopologyEdge) -> String {
    format!(
        "{} <- {}",
        topology_edge_label(edge),
        topology_node_label(&edge.from)
    )
}

fn outgoing_edge_chip_label(edge: &TopologyEdge) -> String {
    format!(
        "{} -> {}",
        topology_edge_label(edge),
        topology_node_label(&edge.to)
    )
}

fn topology_edge_label(edge: &TopologyEdge) -> String {
    if let Some(label) = edge
        .label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
    {
        label.to_owned()
    } else if edge.kind.trim().is_empty() {
        "edge".to_owned()
    } else {
        edge.kind.clone()
    }
}

fn topology_node_label(node_id: &str) -> String {
    if node_id == "config" {
        return "config".to_owned();
    }
    if node_id == "tools" {
        return "ToolRegistry".to_owned();
    }
    if let Some(id) = node_id.strip_prefix("slot:") {
        return format!("slot:{id}");
    }
    if let Some(name) = node_id.strip_prefix("plugin:") {
        return format!("plugin:{name}");
    }
    if let Some(name) = node_id.strip_prefix("tool:") {
        return format!("tool:{name}");
    }
    if let Some(name) = node_id.strip_prefix("context_provider:") {
        return format!("context:{name}");
    }
    if let Some(rest) = node_id.strip_prefix("module:") {
        let mut parts = rest.splitn(2, ':');
        let slot = parts.next().unwrap_or_default();
        let module = parts.next().unwrap_or_default();
        if module.is_empty() {
            return format!("module:{slot}");
        }
        return format!("{slot}/{module}");
    }
    node_id.to_owned()
}

fn topology_plugin_badge_class(status: &str) -> &'static str {
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

#[component]
pub(crate) fn ApprovalCard<F>(request: ApprovalRequestInfo, on_resolve: F) -> impl IntoView
where
    F: Fn(String, bool, ApprovalCacheScope) + Copy + 'static,
{
    let (cache, set_cache) = signal(ApprovalCacheScope::None);
    let approve_id = request.approval_id.clone();
    let deny_id = request.approval_id.clone();
    let args_preview = compact_json(&request.call.args);
    let is_safe_cache_target = approval_allows_tool_cwd_cache(&request);
    let spec_hint = request
        .tool_spec
        .as_ref()
        .and_then(|spec| spec.get("description"))
        .and_then(Value::as_str)
        .unwrap_or(&request.reason)
        .to_owned();

    view! {
        <article class="control-card approval-card">
            <div class="control-card-header">
                <span class="status-badge running">
                    <span class="dot"></span>
                    "Доступ"
                </span>
                <strong>{request.call.name}</strong>
                <code>{short_path(&request.cwd)}</code>
            </div>
            <p>{spec_hint}</p>
            <pre>{args_preview}</pre>
            <div class="control-row">
                <span class="control-label">"Кэш"</span>
                <div class="segmented">
                    <button
                        type="button"
                        class:active=move || cache.get() == ApprovalCacheScope::None
                        on:click=move |_| set_cache.set(ApprovalCacheScope::None)
                    >
                        {ApprovalCacheScope::None.label()}
                    </button>
                    <button
                        type="button"
                        class:active=move || cache.get() == ApprovalCacheScope::ExactCall
                        on:click=move |_| set_cache.set(ApprovalCacheScope::ExactCall)
                    >
                        {ApprovalCacheScope::ExactCall.label()}
                    </button>
                    {if is_safe_cache_target {
                        view! {
                            <button
                                type="button"
                                class:active=move || cache.get() == ApprovalCacheScope::ToolInCwd
                                on:click=move |_| set_cache.set(ApprovalCacheScope::ToolInCwd)
                            >
                                {ApprovalCacheScope::ToolInCwd.label()}
                            </button>
                        }.into_any()
                    } else {
                        view! { <></> }.into_any()
                    }}
                </div>
            </div>
            <div class="control-actions">
                <button
                    type="button"
                    class="secondary danger"
                    on:click=move |_| on_resolve(deny_id.clone(), false, ApprovalCacheScope::None)
                >
                    "Отклонить"
                </button>
                <button
                    type="button"
                    class="btn-primary"
                    on:click=move |_| on_resolve(approve_id.clone(), true, cache.get())
                >
                    "Разрешить"
                </button>
            </div>
        </article>
    }
}

fn approval_allows_tool_cwd_cache(request: &ApprovalRequestInfo) -> bool {
    request
        .tool_spec
        .as_ref()
        .and_then(|spec| spec.get("safety"))
        .and_then(Value::as_str)
        .is_some_and(|safety| matches!(safety, "ReadOnly" | "WritesFiles"))
}

#[component]
pub(crate) fn UserInputCard<F>(request: UserInputRequestInfo, on_submit: F) -> impl IntoView
where
    F: Fn(String, HashMap<String, Vec<String>>) + Copy + 'static,
{
    let (answers, set_answers) = signal(HashMap::<String, Vec<String>>::new());
    let (custom_answers, set_custom_answers) = signal(HashMap::<String, String>::new());
    let (current_question, set_current_question) = signal(0usize);
    let request_id = request.request_id.clone();
    let title = request
        .title
        .clone()
        .unwrap_or_else(|| "Нужен ответ".to_owned());
    let questions = request.questions.clone();
    let question_count = questions.len();
    let tabs = questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let label = if question.header.trim().is_empty() {
                format!("Вопрос {}", index + 1)
            } else {
                question.header.clone()
            };
            (index, label)
        })
        .collect::<Vec<_>>();

    let submit_answers = move || {
        let mut merged = answers.get();
        for (question_id, value) in custom_answers.get() {
            let value = value.trim().to_owned();
            if !value.is_empty() {
                merged.entry(question_id).or_default().push(value);
            }
        }
        on_submit(request_id.clone(), merged);
    };
    let confirm = move |_| {
        if question_count == 0 || current_question.get() + 1 >= question_count {
            submit_answers();
        } else {
            set_current_question.update(|index| *index += 1);
        }
    };

    view! {
        <article class="task-card running input-chat-card">
            <div class="task-card-header input-chat-header">
                <span class="status-badge disconnected">
                    <span class="dot"></span>
                    "Вопрос"
                </span>
                <strong>{title}</strong>
                <span class="input-step">
                    {move || {
                        if question_count == 0 {
                            "0 / 0".to_owned()
                        } else {
                            format!("{} / {}", current_question.get() + 1, question_count)
                        }
                    }}
                </span>
            </div>
            <div class="input-tabs" role="tablist">
                <For
                    each=move || tabs.clone()
                    key=|(index, _)| *index
                    children=move |(index, label)| {
                        view! {
                            <button
                                type="button"
                                role="tab"
                                class=move || {
                                    if current_question.get() == index {
                                        "active"
                                    } else if current_question.get() > index {
                                        "done"
                                    } else {
                                        ""
                                    }
                                }
                                on:click=move |_| set_current_question.set(index)
                            >
                                <span>{label}</span>
                            </button>
                        }
                    }
                />
            </div>
            {move || {
                let Some(question) = questions.get(current_question.get()).cloned() else {
                    return view! {
                        <section class="input-question">
                            <div class="input-question-header">
                                <span>"Вопрос"</span>
                                <strong>"Вопросы не переданы"</strong>
                            </div>
                        </section>
                    }.into_any();
                };
                let question_id = question.id.clone();
                let custom_value_question_id = question.id.clone();
                let custom_write_question_id = question.id.clone();
                let header = question.header.clone();
                let question_text = question.question.clone();
                let options = question.options.clone();
                let multi_select = question.multi_select;
                let show_custom = question.is_other || question.options.is_empty();
                let input_type = if question.is_secret { "password" } else { "text" };

                view! {
                    <section class="input-question">
                        <div class="input-question-header">
                            <span>{header}</span>
                            <strong>{question_text}</strong>
                        </div>
                        <div class="choice-grid">
                            <For
                                each=move || options.clone()
                                key=|option| option.label.clone()
                                children=move |option| {
                                    let selected_question_id = question_id.clone();
                                    let selected_label = option.label.clone();
                                    let click_question_id = question_id.clone();
                                    let click_label = option.label.clone();
                                    view! {
                                        <button
                                            type="button"
                                            class="choice-button"
                                            class:active=move || {
                                                answers
                                                    .get()
                                                    .get(&selected_question_id)
                                                    .is_some_and(|values| values.contains(&selected_label))
                                            }
                                            on:click=move |_| {
                                                let question_id = click_question_id.clone();
                                                let label = click_label.clone();
                                                set_answers.update(|all| {
                                                    if multi_select {
                                                        let values = all.entry(question_id).or_default();
                                                        if let Some(index) = values.iter().position(|value| value == &label) {
                                                            values.remove(index);
                                                        } else {
                                                            values.push(label);
                                                        }
                                                    } else {
                                                        all.insert(question_id, vec![label]);
                                                    }
                                                });
                                            }
                                        >
                                            <span>{option.label}</span>
                                            <small>{option.description}</small>
                                        </button>
                                    }
                                }
                            />
                        </div>
                        {if show_custom {
                            view! {
                                <input
                                    type=input_type
                                    placeholder="Свой вариант"
                                    prop:value=move || {
                                        custom_answers
                                            .get()
                                            .get(&custom_value_question_id)
                                            .cloned()
                                            .unwrap_or_default()
                                    }
                                    on:input:target=move |ev| {
                                        set_custom_answers.update(|answers| {
                                            answers.insert(custom_write_question_id.clone(), ev.target().value());
                                        });
                                    }
                                />
                            }.into_any()
                        } else {
                            view! { <></> }.into_any()
                        }}
                    </section>
                }.into_any()
            }}
            <div class="input-chat-actions">
                <button
                    type="button"
                    class="secondary"
                    disabled=move || current_question.get() == 0
                    on:click=move |_| set_current_question.update(|index| *index = index.saturating_sub(1))
                >
                    "Назад"
                </button>
                <button type="button" class="btn-primary" on:click=confirm>
                    {move || {
                        if question_count == 0 || current_question.get() + 1 >= question_count {
                            "Отправить"
                        } else {
                            "Подтвердить"
                        }
                    }}
                </button>
            </div>
        </article>
    }
}

#[component]
pub(crate) fn ToastStack<F>(toasts: ReadSignal<Vec<ToastMessage>>, on_dismiss: F) -> impl IntoView
where
    F: Fn(u64) + Copy + Send + 'static,
{
    view! {
        <div class="toast-stack" aria-live="polite">
            <For
                each=move || toasts.get()
                key=|toast| toast.id
                children=move |toast| {
                    let toast_id = toast.id;
                    view! {
                        <div class="toast">
                            <span>{toast.text}</span>
                            <button
                                type="button"
                                class="secondary"
                                title="Закрыть"
                                on:click=move |_| on_dismiss(toast_id)
                            >
                                "×"
                            </button>
                        </div>
                    }
                }
            />
        </div>
    }
}

#[component]
pub(crate) fn QueuedPromptCard<S, C>(
    text: String,
    is_sending: ReadSignal<bool>,
    on_send: S,
    on_clear: C,
) -> impl IntoView
where
    S: Fn(MouseEvent) + Copy + 'static,
    C: Fn(MouseEvent) + Copy + 'static,
{
    let preview = text.clone();
    view! {
        <article class="task-card running queued-card">
            <div class="task-card-header">
                <span class="status-badge disconnected">
                    <span class="dot"></span>
                    "В очереди"
                </span>
            </div>
            <div class="message system-message queued-message">
                <p>{preview}</p>
                <div class="queued-actions">
                    <button
                        type="button"
                        class="btn-primary"
                        disabled=move || is_sending.get()
                        on:click=on_send
                    >
                        "Отправить"
                    </button>
                    <button type="button" class="secondary" on:click=on_clear>
                        "Убрать"
                    </button>
                </div>
            </div>
        </article>
    }
}

#[component]
pub(crate) fn PlanActionsCard<R, E, X>(on_revise: R, on_execute: E, on_exit: X) -> impl IntoView
where
    R: Fn(MouseEvent) + Copy + 'static,
    E: Fn(MouseEvent) + Copy + 'static,
    X: Fn(MouseEvent) + Copy + 'static,
{
    view! {
        <article class="task-card running plan-actions-card">
            <div class="task-card-header">
                <span class="status-badge running">
                    <span class="dot"></span>
                    "План готов"
                </span>
            </div>
            <div class="message system-message plan-actions-message">
                <button
                    type="button"
                    class="secondary"
                    on:click=on_revise
                    title="Уточнить последний план текстом из поля ввода"
                >
                    "Уточнить"
                </button>
                <button
                    type="button"
                    class="btn-primary"
                    on:click=on_execute
                    title="Переключиться в обычный режим и выполнить последний план"
                >
                    "Выполнить"
                </button>
                <button
                    type="button"
                    class="secondary"
                    on:click=on_exit
                    title="Вернуться в обычный режим"
                >
                    "Выйти"
                </button>
            </div>
        </article>
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TimelineItem {
    Message(Message),
    AgentChain(Vec<Message>),
}

fn message_timeline_items(messages: Vec<Message>) -> Vec<TimelineItem> {
    fn flush_agent_chain(items: &mut Vec<TimelineItem>, chain: &mut Vec<Message>) {
        if chain.is_empty() {
            return;
        }

        items.push(TimelineItem::AgentChain(std::mem::take(chain)));
    }

    let mut items = Vec::new();
    let mut agent_chain = Vec::new();

    for message in messages {
        if message.role == MessageRole::User {
            flush_agent_chain(&mut items, &mut agent_chain);
            items.push(TimelineItem::Message(message));
        } else {
            agent_chain.push(message);
        }
    }

    flush_agent_chain(&mut items, &mut agent_chain);
    items
}

#[component]
pub(crate) fn MessageTimeline(messages: ReadSignal<Vec<Message>>) -> impl IntoView {
    move || {
        message_timeline_items(messages.get())
            .into_iter()
            .map(|item| match item {
                TimelineItem::Message(message) => view! { <MessageView message /> }.into_any(),
                TimelineItem::AgentChain(messages) => {
                    view! { <AgentChainView messages /> }.into_any()
                }
            })
            .collect::<Vec<_>>()
    }
}

#[component]
fn AgentChainView(messages: Vec<Message>) -> impl IntoView {
    let messages_for_list = messages.clone();
    view! {
        <article class="task-card running agent-chain-card">
            <div class="agent-chain-list">
                <For
                    each=move || messages_for_list.clone()
                    key=|message| message.render_key()
                    children=move |message| view! { <AgentChainItem message /> }
                />
            </div>
        </article>
    }
}

#[component]
fn AgentChainItem(message: Message) -> impl IntoView {
    if let Some(tool) = message.tool {
        return view! {
            <section class="agent-chain-item tool-chain-item">
                <ToolActivityCard tool />
            </section>
        }
        .into_any();
    }

    view! {
        <section class="agent-chain-item message-chain-item">
            <ChainMessageView message />
        </section>
    }
    .into_any()
}

#[component]
fn ChainMessageView(message: Message) -> impl IntoView {
    let message_class = message.role.message_class();
    let badge_class = message.role.badge_class();
    let text = message.text.clone();
    let html = markdown_html(&text);
    let content_class = if message.streaming {
        format!("{message_class} streaming-message")
    } else {
        message_class.to_owned()
    };
    let (collapsed, set_collapsed) = signal(false);
    let copy_text = text.clone();
    let toggle_title = move || {
        if collapsed.get() {
            "Развернуть"
        } else {
            "Свернуть"
        }
    };
    view! {
        <div class="task-card-header agent-chain-item-header">
            <span class=badge_class>
                <span class="dot"></span>
                {message.role.label()}
            </span>
            <div class="message-actions">
                <button
                    type="button"
                    class="icon-button"
                    title="Скопировать markdown"
                    on:click=move |_| copy_to_clipboard(copy_text.clone())
                >
                    "Копировать"
                </button>
                <button
                    type="button"
                    class="icon-button"
                    title=toggle_title
                    on:click=move |_| set_collapsed.update(|value| *value = !*value)
                >
                    {move || if collapsed.get() { "Открыть" } else { "Скрыть" }}
                </button>
            </div>
        </div>
        {move || {
            if collapsed.get() {
                view! {
                    <div class="message collapsed-message">
                        "Сообщение скрыто"
                    </div>
                }.into_any()
            } else {
                view! {
                    <div class=content_class.clone() inner_html=html.clone()></div>
                }.into_any()
            }
        }}
    }
}

#[component]
fn ToolActivityCard(tool: ToolActivity) -> impl IntoView {
    let (expanded, set_expanded) = signal(false);
    let args = tool.args_preview.clone();
    let result = tool.result_preview.clone();
    view! {
        <article class="tool-card">
            <button
                type="button"
                class="tool-card-summary"
                title="Показать детали tool"
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                <span class=tool.status.badge_class()>
                    <span class=if matches!(tool.status, ToolActivityStatus::Running | ToolActivityStatus::WaitingApproval) { "spinner-dot" } else { "dot" }></span>
                    {tool.status.label()}
                </span>
                <strong>{tool.name}</strong>
                <code>{short_id(&tool.call_id).to_owned()}</code>
            </button>
            {move || {
                if expanded.get() {
                    view! {
                        <div class="tool-card-details">
                            <pre>{args.clone()}</pre>
                            {if let Some(result) = result.clone() {
                                view! { <pre>{result}</pre> }.into_any()
                            } else {
                                view! { <></> }.into_any()
                            }}
                        </div>
                    }.into_any()
                } else {
                    view! { <></> }.into_any()
                }
            }}
        </article>
    }
}

#[component]
pub(crate) fn WorkingCard(status: ReadSignal<String>) -> impl IntoView {
    view! {
        <article class="task-card running working-card">
            <div class="task-card-header">
                <span class="status-badge running">
                    <span class="spinner-dot"></span>
                    {move || status.get()}
                </span>
            </div>
        </article>
    }
}

#[component]
pub(crate) fn MessageView(message: Message) -> impl IntoView {
    if message.tool.is_some() {
        return view! { <AgentChainView messages=vec![message] /> }.into_any();
    }

    let card_class = message.role.card_class();
    let message_class = message.role.message_class();
    let badge_class = message.role.badge_class();
    let text = message.text.clone();
    let html = markdown_html(&text);
    let content_class = if message.streaming {
        format!("{message_class} streaming-message")
    } else {
        message_class.to_owned()
    };
    let (collapsed, set_collapsed) = signal(false);
    let copy_text = text.clone();
    let toggle_title = move || {
        if collapsed.get() {
            "Развернуть"
        } else {
            "Свернуть"
        }
    };
    view! {
        <article class=card_class>
            <div class="task-card-header">
                <span class=badge_class>
                    <span class="dot"></span>
                    {message.role.label()}
                </span>
                <div class="message-actions">
                    <button
                        type="button"
                        class="icon-button"
                        title="Скопировать markdown"
                        on:click=move |_| copy_to_clipboard(copy_text.clone())
                    >
                        "Копировать"
                    </button>
                    <button
                        type="button"
                        class="icon-button"
                        title=toggle_title
                        on:click=move |_| set_collapsed.update(|value| *value = !*value)
                    >
                        {move || if collapsed.get() { "Открыть" } else { "Скрыть" }}
                    </button>
                </div>
            </div>
            {move || {
                if collapsed.get() {
                    view! {
                        <div class="message collapsed-message">
                            "Сообщение скрыто"
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class=content_class.clone() inner_html=html.clone()></div>
                    }.into_any()
                }
            }}
        </article>
    }
    .into_any()
}
