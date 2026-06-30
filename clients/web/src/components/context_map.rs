use leptos::{prelude::*, task::spawn_local};

use super::format_token_count;
use crate::api::{encode_query_component, get_json};
use crate::app_helpers::sidebar_session_title;
use crate::types::*;
use crate::ui_utils::{short_id, short_path};

#[component]
pub(crate) fn ContextMapView(
    sessions: ReadSignal<Vec<SessionSummary>>,
    active_session_dir: ReadSignal<Option<String>>,
) -> impl IntoView {
    let (selected_session_dir, set_selected_session_dir) =
        signal(active_session_dir.get_untracked());
    let (snapshot, set_snapshot) = signal(None::<ContextMapSnapshot>);
    let (status, set_status) = signal("загружаю карту контекста".to_owned());

    load_context_map_snapshot(
        selected_session_dir.get_untracked(),
        set_snapshot,
        set_status,
    );

    let refresh = move |_| {
        load_context_map_snapshot(
            selected_session_dir.get_untracked(),
            set_snapshot,
            set_status,
        );
    };
    let select_session = move |session_dir: Option<String>| {
        set_selected_session_dir.set(session_dir.clone());
        load_context_map_snapshot(session_dir, set_snapshot, set_status);
    };

    view! {
        <section class="context-page">
            <div class="resume-toolbar context-toolbar">
                <div>
                    <h2>"Карта контекста"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <div class="context-toolbar-actions">
                    <select
                        class="context-session-select"
                        prop:value=move || selected_session_dir.get().unwrap_or_default()
                        on:change:target=move |ev| {
                            let value = ev.target().value();
                            if value.trim().is_empty() {
                                select_session(None);
                            } else {
                                select_session(Some(value));
                            }
                        }
                    >
                        <option value="">"Текущая session"</option>
                        <For
                            each=move || sessions.get()
                            key=|session| session.session_dir.clone()
                            children=move |session| {
                                let label = context_session_option_label(&session);
                                view! {
                                    <option value=session.session_dir.clone()>{label}</option>
                                }
                            }
                        />
                    </select>
                    <button type="button" class="secondary" on:click=refresh>"Обновить"</button>
                </div>
            </div>
            {move || {
                match snapshot.get() {
                    Some(snapshot) => context_snapshot_view(snapshot).into_any(),
                    None => view! {
                        <div class="empty-state">
                            <div class="empty-state-title">{move || status.get()}</div>
                        </div>
                    }.into_any(),
                }
            }}
        </section>
    }
}

fn load_context_map_snapshot(
    session_dir: Option<String>,
    set_snapshot: WriteSignal<Option<ContextMapSnapshot>>,
    set_status: WriteSignal<String>,
) {
    set_status.set("загружаю карту контекста".to_owned());
    spawn_local(async move {
        match get_json::<ContextMapSnapshot>(&context_map_path(session_dir.as_deref())).await {
            Ok(snapshot) => {
                let label = snapshot
                    .session_dir
                    .as_deref()
                    .map(short_path)
                    .unwrap_or_else(|| "текущая session".to_owned());
                set_status.set(format!("snapshot: {label}"));
                set_snapshot.set(Some(snapshot));
            }
            Err(error) => {
                set_status.set(format!("не удалось загрузить карту: {error}"));
                set_snapshot.set(None);
            }
        }
    });
}

fn context_map_path(session_dir: Option<&str>) -> String {
    match session_dir {
        Some(session_dir) => format!(
            "/context?session_dir={}",
            encode_query_component(session_dir)
        ),
        None => "/context".to_owned(),
    }
}

fn context_session_option_label(session: &SessionSummary) -> String {
    let id = session
        .session_id
        .as_deref()
        .map(short_id)
        .unwrap_or("legacy");
    let title = sidebar_session_title(session);
    format!("{title} · {id}")
}

fn context_snapshot_view(snapshot: ContextMapSnapshot) -> impl IntoView {
    let used_tokens = context_used_tokens(&snapshot);
    let max_tokens = snapshot
        .latest_usage
        .as_ref()
        .and_then(|usage| usage.max_input_tokens);
    let free_tokens = max_tokens.map(|max| max.saturating_sub(used_tokens));
    let usage_percent = max_tokens
        .filter(|max| *max > 0)
        .map(|max| ((f64::from(used_tokens) / f64::from(max)) * 100.0).round() as u32);
    let usage = snapshot.latest_usage.clone();
    let history = snapshot.history.clone();
    let latest_context = snapshot.latest_context.clone();
    let latest_compaction = snapshot.latest_compaction.clone();
    let tools = snapshot.tools.clone();
    let diagnostics = snapshot.diagnostics.clone();
    let session_path = snapshot
        .session_dir
        .as_deref()
        .map(short_path)
        .unwrap_or_else(|| "current".to_owned());
    let workspace = snapshot
        .workspace_path
        .clone()
        .unwrap_or_else(|| "workspace unknown".to_owned());
    let activity = snapshot
        .activity
        .as_ref()
        .map(context_activity_label)
        .unwrap_or_else(|| "cold".to_owned());
    let source = usage
        .as_ref()
        .map(|usage| usage.source.clone())
        .unwrap_or_else(|| "history".to_owned());
    let cache = context_cache_view_model(usage.as_ref());
    let metrics = vec![
        (
            "used".to_owned(),
            format_token_count(used_tokens),
            usage_percent
                .map(|percent| format!("{percent}% окна"))
                .unwrap_or_else(|| "window unknown".to_owned()),
        ),
        (
            "free".to_owned(),
            free_tokens
                .map(format_token_count)
                .unwrap_or_else(|| "n/a".to_owned()),
            max_tokens
                .map(|max| format!("из {}", format_token_count(max)))
                .unwrap_or_else(|| "max_input_tokens не задан".to_owned()),
        ),
        (
            "cache hit".to_owned(),
            cache.hit_rate.clone(),
            "provider input cache".to_owned(),
        ),
        (
            "cache".to_owned(),
            cache.status.clone(),
            cache.status_detail.clone(),
        ),
    ];
    let categories = usage
        .as_ref()
        .map(|usage| usage.categories.clone())
        .unwrap_or_default();
    let category_total = categories
        .iter()
        .map(|category| category.tokens)
        .sum::<u32>()
        .max(1);
    let tool_names = if tools.names.is_empty() {
        "нет tool events".to_owned()
    } else {
        tools.names.join(", ")
    };

    view! {
        <div class="context-map-scroll">
            <section class="context-overview">
                <For
                    each=move || metrics.clone()
                    key=|metric| metric.0.clone()
                    children=move |(label, value, detail)| {
                        view! {
                            <div class="context-metric">
                                <span>{label}</span>
                                <strong>{value}</strong>
                                <small>{detail}</small>
                            </div>
                        }
                    }
                />
            </section>

            <section class="context-grid">
                <article class="context-panel context-panel-wide">
                    <div class="context-panel-header">
                        <div>
                            <span class="panel-kicker">"Usage"</span>
                            <h3>{context_usage_title(usage.as_ref())}</h3>
                        </div>
                        <span class="status-badge idle">{source}</span>
                    </div>
                    {if categories.is_empty() {
                        view! {
                            <div class="context-empty-line">"Нет category breakdown для этой session"</div>
                        }.into_any()
                    } else {
                        view! {
                            <div class="context-category-list">
                                <For
                                    each=move || categories.clone()
                                    key=|category| category.name.clone()
                                    children=move |category| {
                                        let percent = ((f64::from(category.tokens) / f64::from(category_total)) * 100.0)
                                            .round()
                                            .clamp(0.0, 100.0);
                                        let token_label = match category.source.as_deref() {
                                            Some(source) => format!(
                                                "{} · {}",
                                                format_token_count(category.tokens),
                                                context_category_source_label(source)
                                            ),
                                            None => format_token_count(category.tokens),
                                        };
                                        view! {
                                            <div class="context-category-row">
                                                <div class="context-category-head">
                                                    <span>{context_category_label(&category.name)}</span>
                                                    <code>{token_label}</code>
                                                </div>
                                                <div class="context-category-bar">
                                                    <span style=format!("width: {percent:.0}%")></span>
                                                </div>
                                            </div>
                                        }
                                    }
                                />
                            </div>
                        }.into_any()
                    }}
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"Session"</span>
                    <dl class="context-kv">
                        <div><dt>"session"</dt><dd>{session_path}</dd></div>
                        <div><dt>"workspace"</dt><dd title=workspace.clone()>{short_path(&workspace)}</dd></div>
                        <div><dt>"activity"</dt><dd>{activity}</dd></div>
                        <div><dt>"source"</dt><dd>{context_source_label(usage.as_ref())}</dd></div>
                    </dl>
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"History"</span>
                    <dl class="context-kv">
                        <div><dt>"messages"</dt><dd>{history.messages.to_string()}</dd></div>
                        <div><dt>"user"</dt><dd>{history.user_messages.to_string()}</dd></div>
                        <div><dt>"assistant"</dt><dd>{history.assistant_messages.to_string()}</dd></div>
                        <div><dt>"tool results"</dt><dd>{history.tool_results.to_string()}</dd></div>
                        <div><dt>"estimate"</dt><dd>{format_token_count(history.estimated_tokens)}</dd></div>
                    </dl>
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"Ephemeral Context"</span>
                    <dl class="context-kv">
                        <div><dt>"chunks"</dt><dd>{latest_context.as_ref().map(|context| context.chunks.to_string()).unwrap_or_else(|| "n/a".to_owned())}</dd></div>
                        <div><dt>"tokens"</dt><dd>{latest_context.as_ref().and_then(|context| context.token_estimate).map(format_token_count).unwrap_or_else(|| "n/a".to_owned())}</dd></div>
                        <div><dt>"turn"</dt><dd>{latest_context.as_ref().and_then(|context| context.turn_id.as_deref()).map(short_id).unwrap_or("n/a").to_owned()}</dd></div>
                    </dl>
                </article>

                <article class="context-panel">
                    <div class="context-panel-header">
                        <div>
                            <span class="panel-kicker">"Provider Input Cache"</span>
                        </div>
                        <span class=cache.badge_class.clone()>
                            <span class="dot"></span>
                            {cache.status.clone()}
                        </span>
                    </div>
                    <dl class="context-kv">
                        <div><dt>"input"</dt><dd>{cache.input_tokens.clone()}</dd></div>
                        <div><dt>"cached"</dt><dd>{cache.cached_input_tokens.clone()}</dd></div>
                        <div><dt>"created"</dt><dd>{cache.cache_creation_input_tokens.clone()}</dd></div>
                        <div><dt>"hit rate"</dt><dd>{cache.hit_rate.clone()}</dd></div>
                    </dl>
                    <div class="context-cache-bar" title=cache.hit_title.clone()>
                        <span style=format!("width: {}%", cache.hit_percent)></span>
                    </div>
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"Tools"</span>
                    <dl class="context-kv">
                        <div><dt>"requested"</dt><dd>{tools.requested.to_string()}</dd></div>
                        <div><dt>"finished"</dt><dd>{tools.finished.to_string()}</dd></div>
                        <div><dt>"failed"</dt><dd>{tools.failed.to_string()}</dd></div>
                    </dl>
                    <p class="context-muted-line">{tool_names}</p>
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"Compaction"</span>
                    {context_compaction_view(latest_compaction).into_any()}
                </article>

                <article class="context-panel context-panel-wide">
                    <span class="panel-kicker">"Diagnostics"</span>
                    {if diagnostics.is_empty() {
                        view! { <div class="context-empty-line">"Нет предупреждений"</div> }.into_any()
                    } else {
                        view! {
                            <ul class="context-diagnostics">
                                <For
                                    each=move || diagnostics.clone()
                                    key=|item| item.clone()
                                    children=move |item| view! { <li>{item}</li> }
                                />
                            </ul>
                        }.into_any()
                    }}
                </article>
            </section>
        </div>
    }
}

fn context_used_tokens(snapshot: &ContextMapSnapshot) -> u32 {
    snapshot
        .latest_usage
        .as_ref()
        .and_then(|usage| usage.actual.as_ref().map(|actual| actual.input_tokens))
        .or_else(|| {
            snapshot
                .latest_usage
                .as_ref()
                .map(|usage| usage.estimated_input_tokens)
        })
        .unwrap_or(snapshot.history.estimated_tokens)
}

fn context_usage_title(usage: Option<&ContextUsageSnapshot>) -> String {
    let Some(usage) = usage else {
        return "history estimate".to_owned();
    };
    let phase = usage.phase.as_deref().unwrap_or("request");
    format!("{}/{} · {phase}", usage.model_provider, usage.model_name)
}

fn context_source_label(usage: Option<&ContextUsageSnapshot>) -> String {
    let Some(usage) = usage else {
        return "history fallback".to_owned();
    };
    match usage.source.as_str() {
        "mixed" => "provider totals + local estimates".to_owned(),
        "provider" => "provider totals".to_owned(),
        "estimated" => "local estimate".to_owned(),
        other => other.to_owned(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ContextCacheViewModel {
    status: String,
    status_detail: String,
    badge_class: String,
    input_tokens: String,
    cached_input_tokens: String,
    cache_creation_input_tokens: String,
    hit_rate: String,
    hit_title: String,
    hit_percent: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContextCacheStatus {
    Unavailable,
    Cold,
    Warming,
    Hot,
}

impl ContextCacheStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "n/a",
            Self::Cold => "cold",
            Self::Warming => "warming",
            Self::Hot => "hot",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Unavailable => "provider usage missing",
            Self::Cold => "no cache read",
            Self::Warming => "cache warming",
            Self::Hot => "cache read active",
        }
    }

    fn badge_class(self) -> &'static str {
        match self {
            Self::Unavailable | Self::Cold => "status-badge idle",
            Self::Warming => "status-badge disconnected",
            Self::Hot => "status-badge completed",
        }
    }
}

fn context_cache_view_model(usage: Option<&ContextUsageSnapshot>) -> ContextCacheViewModel {
    let Some(actual) = usage.and_then(|usage| usage.actual.as_ref()) else {
        return ContextCacheViewModel::from_values(ContextCacheStatus::Unavailable, 0, 0, 0, None);
    };
    let input_tokens = actual.input_tokens;
    let cached_input_tokens = actual.cached_input_tokens.unwrap_or(0);
    let cache_creation_input_tokens = actual.cache_creation_input_tokens.unwrap_or(0);
    let hit_percent = context_cache_hit_percent(input_tokens, cached_input_tokens);
    let status = context_cache_status(
        input_tokens,
        cached_input_tokens,
        cache_creation_input_tokens,
        hit_percent,
    );
    ContextCacheViewModel::from_values(
        status,
        input_tokens,
        cached_input_tokens,
        cache_creation_input_tokens,
        hit_percent,
    )
}

impl ContextCacheViewModel {
    fn from_values(
        status: ContextCacheStatus,
        input_tokens: u32,
        cached_input_tokens: u32,
        cache_creation_input_tokens: u32,
        hit_percent: Option<u32>,
    ) -> Self {
        let hit_rate = hit_percent
            .map(|percent| format!("{percent}%"))
            .unwrap_or_else(|| "n/a".to_owned());
        let hit_title = if input_tokens == 0 {
            "no provider input usage".to_owned()
        } else {
            format!(
                "{} cached / {} input",
                format_token_count(cached_input_tokens),
                format_token_count(input_tokens)
            )
        };
        Self {
            status: status.label().to_owned(),
            status_detail: status.detail().to_owned(),
            badge_class: status.badge_class().to_owned(),
            input_tokens: optional_token_count(
                input_tokens,
                status != ContextCacheStatus::Unavailable,
            ),
            cached_input_tokens: optional_token_count(
                cached_input_tokens,
                status != ContextCacheStatus::Unavailable,
            ),
            cache_creation_input_tokens: optional_token_count(
                cache_creation_input_tokens,
                status != ContextCacheStatus::Unavailable,
            ),
            hit_rate,
            hit_title,
            hit_percent: hit_percent.unwrap_or(0),
        }
    }
}

fn context_cache_status(
    input_tokens: u32,
    cached_input_tokens: u32,
    cache_creation_input_tokens: u32,
    hit_percent: Option<u32>,
) -> ContextCacheStatus {
    if input_tokens == 0 {
        return ContextCacheStatus::Unavailable;
    }
    if cached_input_tokens == 0 && cache_creation_input_tokens == 0 {
        return ContextCacheStatus::Cold;
    }
    if hit_percent.is_some_and(|percent| percent >= 50) {
        ContextCacheStatus::Hot
    } else {
        ContextCacheStatus::Warming
    }
}

fn context_cache_hit_percent(input_tokens: u32, cached_input_tokens: u32) -> Option<u32> {
    if input_tokens == 0 {
        return None;
    }
    Some(
        ((f64::from(cached_input_tokens) / f64::from(input_tokens)) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u32,
    )
}

fn optional_token_count(tokens: u32, available: bool) -> String {
    if available {
        format_token_count(tokens)
    } else {
        "n/a".to_owned()
    }
}

fn context_category_label(name: &str) -> String {
    match name {
        "instructions" => "instructions".to_owned(),
        "messages" => "messages/history".to_owned(),
        "context" => "ephemeral context".to_owned(),
        "tool_calls" => "tool calls".to_owned(),
        "tool_results" => "tool results".to_owned(),
        "files" => "files".to_owned(),
        "patches" => "patches".to_owned(),
        "tool_schemas" => "tool schemas".to_owned(),
        "provider_cache_read" => "provider cache read".to_owned(),
        "provider_cache_write" => "provider cache write".to_owned(),
        other => other.replace('_', " "),
    }
}

fn context_category_source_label(source: &str) -> String {
    match source {
        "estimated" => "estimate".to_owned(),
        "provider" => "provider".to_owned(),
        "mixed" => "mixed".to_owned(),
        other => other.replace('_', " "),
    }
}

fn context_activity_label(activity: &SessionActivityInfo) -> String {
    if activity.running_turns > 0 {
        format!("{} · {} turns", activity.status, activity.running_turns)
    } else if activity.pending_approvals > 0 {
        format!("{} · approvals", activity.status)
    } else if activity.pending_user_inputs > 0 {
        format!("{} · input", activity.status)
    } else {
        activity.status.clone()
    }
}

fn context_compaction_view(compaction: Option<ContextCompactionSnapshot>) -> impl IntoView {
    match compaction {
        Some(compaction) => {
            let status = compaction.status;
            let report = compaction.report;
            let summary = if compaction.summary_present {
                "summary stored, content hidden".to_owned()
            } else {
                "no summary text".to_owned()
            };
            view! {
                <dl class="context-kv">
                    <div><dt>"status"</dt><dd>{status}</dd></div>
                    <div><dt>"summary"</dt><dd>{summary}</dd></div>
                    {match report {
                        Some(report) => view! {
                            <>
                                <div><dt>"changed"</dt><dd>{report.changed.to_string()}</dd></div>
                                <div><dt>"messages"</dt><dd>{format!("{} -> {}", report.input_messages, report.output_messages)}</dd></div>
                                <div><dt>"tokens"</dt><dd>{context_compaction_tokens(&report)}</dd></div>
                            </>
                        }.into_any(),
                        None => ().into_any(),
                    }}
                </dl>
            }
            .into_any()
        }
        None => view! {
            <div class="context-empty-line">"Compaction events не найдены"</div>
        }
        .into_any(),
    }
}

fn context_compaction_tokens(report: &ContextCompactionReport) -> String {
    match (report.original_token_estimate, report.output_token_estimate) {
        (Some(before), Some(after)) => {
            format!(
                "{} -> {}",
                format_token_count(before),
                format_token_count(after)
            )
        }
        (Some(before), None) => format_token_count(before),
        _ => "n/a".to_owned(),
    }
}
