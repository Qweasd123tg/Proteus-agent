use super::*;
use super::{cache::*, map::*};
use crate::ui_utils::short_id;

pub(super) fn context_snapshot_view(snapshot: ContextMapSnapshot) -> impl IntoView {
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
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
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
            "занято".to_owned(),
            format_token_count(used_tokens),
            usage_percent
                .map(|percent| format!("{percent}% окна"))
                .unwrap_or_else(|| "размер окна неизвестен".to_owned()),
        ),
        (
            "свободно".to_owned(),
            free_tokens
                .map(format_token_count)
                .unwrap_or_else(|| "n/a".to_owned()),
            max_tokens
                .map(|max| format!("из {}", format_token_count(max)))
                .unwrap_or_else(|| "max_input_tokens не задан".to_owned()),
        ),
        (
            "кэш-хиты".to_owned(),
            cache.hit_rate.clone(),
            "входной кэш провайдера".to_owned(),
        ),
        (
            "кэш".to_owned(),
            cache.status.clone(),
            cache.status_detail.clone(),
        ),
    ];
    let categories = usage
        .as_ref()
        .map(|usage| usage.categories.clone())
        .unwrap_or_default();
    let trigger_tokens = usage
        .as_ref()
        .and_then(|usage| usage.compaction_trigger_tokens);
    let segments = context_map_segments(&categories, used_tokens, max_tokens, trigger_tokens);
    let map_cells = context_map_cell_views(&segments);
    let legend_rows = context_map_legend_views(&segments);
    let tool_names = if tools.names.is_empty() {
        "нет tool-событий".to_owned()
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
                            <span class="panel-kicker">"Окно контекста"</span>
                            <h3>{context_usage_title(usage.as_ref())}</h3>
                        </div>
                        <span class="status-badge idle">{source}</span>
                    </div>
                    {if segments.is_empty() {
                        view! {
                            <div class="context-empty-line">"Для этой сессии ещё нет замеров использования"</div>
                        }.into_any()
                    } else {
                        view! {
                            <div class="context-map-layout">
                                <div class="context-map-grid" role="img" aria-label="Карта окна контекста">
                                    {map_cells}
                                </div>
                                <div class="context-map-legend">
                                    {legend_rows}
                                </div>
                            </div>
                        }.into_any()
                    }}
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"Сессия"</span>
                    <dl class="context-kv">
                        <div><dt>"сессия"</dt><dd>{session_path}</dd></div>
                        <div><dt>"workspace"</dt><dd title=workspace.clone()>{short_path(&workspace)}</dd></div>
                        <div><dt>"активность"</dt><dd>{activity}</dd></div>
                        <div><dt>"источник"</dt><dd>{context_source_label(usage.as_ref())}</dd></div>
                    </dl>
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"История"</span>
                    <dl class="context-kv">
                        <div><dt>"сообщений"</dt><dd>{history.messages.to_string()}</dd></div>
                        <div><dt>"от меня"</dt><dd>{history.user_messages.to_string()}</dd></div>
                        <div><dt>"от агента"</dt><dd>{history.assistant_messages.to_string()}</dd></div>
                        <div><dt>"tool-результаты"</dt><dd>{history.tool_results.to_string()}</dd></div>
                        <div><dt>"оценка"</dt><dd>{format_token_count(history.estimated_tokens)}</dd></div>
                    </dl>
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"Контекст хода"</span>
                    <dl class="context-kv">
                        <div><dt>"чанков"</dt><dd>{latest_context.as_ref().map(|context| context.chunks.to_string()).unwrap_or_else(|| "n/a".to_owned())}</dd></div>
                        <div><dt>"токенов"</dt><dd>{latest_context.as_ref().and_then(|context| context.token_estimate).map(format_token_count).unwrap_or_else(|| "n/a".to_owned())}</dd></div>
                        <div><dt>"ход"</dt><dd>{latest_context.as_ref().and_then(|context| context.turn_id.as_ref()).map(short_id).unwrap_or_else(|| "n/a".into()).to_owned()}</dd></div>
                    </dl>
                </article>

                <article class="context-panel">
                    <div class="context-panel-header">
                        <div>
                            <span class="panel-kicker">"Кэш провайдера"</span>
                        </div>
                        <span class=cache.badge_class.clone()>
                            <span class="dot"></span>
                            {cache.status.clone()}
                        </span>
                    </div>
                    <dl class="context-kv">
                        <div><dt>"input"</dt><dd>{cache.input_tokens.clone()}</dd></div>
                        <div><dt>"из кэша"</dt><dd>{cache.cached_input_tokens.clone()}</dd></div>
                        <div><dt>"записано"</dt><dd>{cache.cache_creation_input_tokens.clone()}</dd></div>
                        <div><dt>"hit rate"</dt><dd>{cache.hit_rate.clone()}</dd></div>
                    </dl>
                    <div class="context-cache-bar" title=cache.hit_title.clone()>
                        <span style=format!("width: {}%", cache.hit_percent)></span>
                    </div>
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"Инструменты"</span>
                    <dl class="context-kv">
                        <div><dt>"запущено"</dt><dd>{tools.requested.to_string()}</dd></div>
                        <div><dt>"завершено"</dt><dd>{tools.finished.to_string()}</dd></div>
                        <div><dt>"с ошибкой"</dt><dd>{tools.failed.to_string()}</dd></div>
                    </dl>
                    <p class="context-muted-line">{tool_names}</p>
                </article>

                <article class="context-panel">
                    <span class="panel-kicker">"Компакция"</span>
                    {context_compaction_view(latest_compaction).into_any()}
                </article>

                <article class="context-panel context-panel-wide">
                    <span class="panel-kicker">"Диагностика"</span>
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

fn context_activity_label(activity: &SessionActivityInfo) -> String {
    if activity.running_runs > 0 {
        format!(
            "{} · {} runs",
            activity.status.as_str(),
            activity.running_runs
        )
    } else if activity.pending_approvals > 0 {
        format!("{} · approvals", activity.status.as_str())
    } else if activity.pending_user_inputs > 0 {
        format!("{} · input", activity.status.as_str())
    } else {
        activity.status.as_str().to_owned()
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
