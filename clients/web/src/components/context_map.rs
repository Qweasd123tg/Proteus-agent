use leptos::{prelude::*, task::spawn_local};

use super::format_token_count;
use crate::api::{encode_query_component, get_json};
use crate::app_helpers::sidebar_session_title;
use crate::types::*;
use crate::ui_utils::{set_timeout, short_id, short_path};

/// Сколько ячеек в цветной карте окна: 20 колонок × 10 рядов.
const CONTEXT_MAP_CELLS: usize = 200;
/// Период автообновления снапшота: страница живёт, как и чат, без ручного
/// «Обновить» (кнопка остаётся для нетерпеливых).
const CONTEXT_REFRESH_MS: i32 = 7000;

/// Сегмент карты окна: категория содержимого либо служебная зона
/// (резерв автокомпакта, свободное место).
#[derive(Clone, Debug, PartialEq)]
struct ContextMapSegment {
    label: String,
    tokens: u32,
    color: &'static str,
    percent: f64,
}

const SEGMENT_FALLBACK_PALETTE: [&str; 6] = [
    "#6b9eff", "#5cc784", "#d8a21e", "#c08cf0", "#5cc0d8", "#ef8fb7",
];
const FREE_COLOR: &str = "#2e2e2e";
const BUFFER_COLOR: &str = "#54383d";

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
    schedule_context_refresh(selected_session_dir, set_snapshot, set_status);

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
                    .unwrap_or_else(|| "текущая сессия".to_owned());
                set_status.set(format!("снапшот: {label}"));
                set_snapshot.set(Some(snapshot));
            }
            Err(error) => {
                set_status.set(format!("не удалось загрузить карту: {error}"));
                set_snapshot.set(None);
            }
        }
    });
}

/// Тихое автообновление: перечитывает снапшот без «загружаю…»-статуса и не
/// затирает ленту при ошибке разовой выборки (сеть мигнула — старые данные
/// полезнее пустой страницы).
fn schedule_context_refresh(
    selected_session_dir: ReadSignal<Option<String>>,
    set_snapshot: WriteSignal<Option<ContextMapSnapshot>>,
    set_status: WriteSignal<String>,
) {
    set_timeout(CONTEXT_REFRESH_MS, move || {
        let session_dir = selected_session_dir.get_untracked();
        spawn_local(async move {
            if let Ok(snapshot) =
                get_json::<ContextMapSnapshot>(&context_map_path(session_dir.as_deref())).await
            {
                // Пока запрос летел, могли выбрать другую сессию — не затираем.
                if selected_session_dir.get_untracked() == session_dir {
                    set_snapshot.set(Some(snapshot));
                }
            }
            schedule_context_refresh(selected_session_dir, set_snapshot, set_status);
        });
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
                        <div><dt>"ход"</dt><dd>{latest_context.as_ref().and_then(|context| context.turn_id.as_deref()).map(short_id).unwrap_or("n/a").to_owned()}</dd></div>
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

/// Сегменты карты окна: категории содержимого (масштабированные к фактическому
/// input — сумма локальных оценок может с ним расходиться), затем свободное
/// место и резерв автокомпакта в хвосте окна.
fn context_map_segments(
    categories: &[ContextUsageCategory],
    used_tokens: u32,
    max_tokens: Option<u32>,
    trigger_tokens: Option<u32>,
) -> Vec<ContextMapSegment> {
    // Кэш-категории провайдера пересекаются с обычным input: на карте окна они
    // задвоили бы занятое. Их место — в панели кэша.
    let content: Vec<&ContextUsageCategory> = categories
        .iter()
        .filter(|category| category.tokens > 0 && !category.name.starts_with("provider_cache"))
        .collect();
    let content_total: u64 = content
        .iter()
        .map(|category| u64::from(category.tokens))
        .sum();

    let mut segments = Vec::new();
    if content_total == 0 {
        if used_tokens > 0 {
            segments.push(ContextMapSegment {
                label: "занято".to_owned(),
                tokens: used_tokens,
                color: SEGMENT_FALLBACK_PALETTE[0],
                percent: 0.0,
            });
        }
    } else {
        for (index, category) in content.iter().enumerate() {
            let tokens =
                (u64::from(used_tokens) * u64::from(category.tokens) / content_total) as u32;
            segments.push(ContextMapSegment {
                label: context_category_label(&category.name),
                tokens,
                color: context_category_color(&category.name, index),
                percent: 0.0,
            });
        }
    }

    if let Some(max) = max_tokens.filter(|max| *max > 0) {
        let buffer = trigger_tokens
            .filter(|trigger| *trigger < max)
            .map(|trigger| max - trigger)
            .unwrap_or(0);
        let free = max.saturating_sub(used_tokens).saturating_sub(buffer);
        if free > 0 {
            segments.push(ContextMapSegment {
                label: "свободно".to_owned(),
                tokens: free,
                color: FREE_COLOR,
                percent: 0.0,
            });
        }
        if buffer > 0 {
            segments.push(ContextMapSegment {
                label: "резерв автокомпакта".to_owned(),
                tokens: buffer,
                color: BUFFER_COLOR,
                percent: 0.0,
            });
        }
    }

    let basis = max_tokens
        .filter(|max| *max > 0)
        .map(u64::from)
        .unwrap_or_else(|| {
            segments
                .iter()
                .map(|segment| u64::from(segment.tokens))
                .sum::<u64>()
                .max(1)
        });
    for segment in &mut segments {
        segment.percent = f64::from(segment.tokens) / basis as f64 * 100.0;
    }
    segments
}

/// Распределение ячеек карты по сегментам: метод наибольших остатков, ненулевой
/// сегмент получает минимум одну ячейку (иначе тонкая категория исчезает).
fn allocate_map_cells(tokens: &[u32], total_cells: usize) -> Vec<usize> {
    let total: u64 = tokens.iter().copied().map(u64::from).sum();
    if total == 0 || total_cells == 0 {
        return vec![0; tokens.len()];
    }

    let mut cells = Vec::with_capacity(tokens.len());
    let mut remainders = Vec::with_capacity(tokens.len());
    let mut allocated = 0usize;
    for (index, item) in tokens.iter().enumerate() {
        let exact = u64::from(*item) as f64 * total_cells as f64 / total as f64;
        let floor = exact.floor() as usize;
        cells.push(floor);
        allocated += floor;
        remainders.push((index, exact - exact.floor()));
    }
    remainders.sort_by(|left, right| right.1.total_cmp(&left.1));
    for (index, _) in remainders
        .into_iter()
        .take(total_cells.saturating_sub(allocated))
    {
        cells[index] += 1;
    }

    for index in 0..tokens.len() {
        if tokens[index] > 0 && cells[index] == 0 {
            let Some(largest) = (0..cells.len())
                .filter(|other| cells[*other] > 1)
                .max_by_key(|other| cells[*other])
            else {
                continue;
            };
            cells[largest] -= 1;
            cells[index] += 1;
        }
    }
    cells
}

fn context_map_cell_views(segments: &[ContextMapSegment]) -> Vec<AnyView> {
    let tokens: Vec<u32> = segments.iter().map(|segment| segment.tokens).collect();
    let cells = allocate_map_cells(&tokens, CONTEXT_MAP_CELLS);
    let mut views = Vec::with_capacity(CONTEXT_MAP_CELLS);
    for (segment, count) in segments.iter().zip(cells) {
        let title = format!(
            "{} · {} · {:.0}%",
            segment.label,
            format_token_count(segment.tokens),
            segment.percent,
        );
        for _ in 0..count {
            views.push(
                view! {
                    <span
                        class="context-map-cell"
                        style=format!("background: {}", segment.color)
                        title=title.clone()
                    ></span>
                }
                .into_any(),
            );
        }
    }
    views
}

fn context_map_legend_views(segments: &[ContextMapSegment]) -> Vec<AnyView> {
    segments
        .iter()
        .map(|segment| {
            view! {
                <div class="context-legend-row">
                    <span
                        class="context-legend-dot"
                        style=format!("background: {}", segment.color)
                    ></span>
                    <span class="context-legend-label">{segment.label.clone()}</span>
                    <code>{format_token_count(segment.tokens)}</code>
                    <code class="context-legend-percent">{format!("{:.0}%", segment.percent)}</code>
                </div>
            }
            .into_any()
        })
        .collect()
}

fn context_category_color(name: &str, index: usize) -> &'static str {
    match name {
        "instructions" => "#6b9eff",
        "messages" => "#5cc784",
        "context" => "#c08cf0",
        "tool_calls" => "#e0975c",
        "tool_results" => "#d8a21e",
        "tool_schemas" => "#5cc0d8",
        "files" => "#7fd4b2",
        "patches" => "#ef8fb7",
        _ => SEGMENT_FALLBACK_PALETTE[index % SEGMENT_FALLBACK_PALETTE.len()],
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

#[cfg(test)]
mod tests {
    use super::*;

    fn category(name: &str, tokens: u32) -> ContextUsageCategory {
        ContextUsageCategory {
            name: name.to_owned(),
            tokens,
            source: None,
        }
    }

    #[test]
    fn allocate_map_cells_sums_to_total_and_keeps_thin_segments_visible() {
        let cells = allocate_map_cells(&[1, 999, 0, 500], 200);

        assert_eq!(cells.iter().sum::<usize>(), 200);
        // Тонкий ненулевой сегмент виден минимум одной ячейкой.
        assert!(cells[0] >= 1);
        // Нулевой не занимает место.
        assert_eq!(cells[2], 0);
    }

    #[test]
    fn context_map_segments_add_free_space_and_autocompact_buffer() {
        let categories = vec![category("instructions", 30), category("messages", 70)];
        let segments = context_map_segments(&categories, 100, Some(200), Some(160));

        let labels: Vec<&str> = segments
            .iter()
            .map(|segment| segment.label.as_str())
            .collect();
        assert_eq!(
            labels,
            vec![
                "instructions",
                "messages/history",
                "свободно",
                "резерв автокомпакта"
            ]
        );
        // 100 занято, до порога 160 свободно 60, резерв 200-160 = 40.
        assert_eq!(segments[2].tokens, 60);
        assert_eq!(segments[3].tokens, 40);
        // Проценты считаются от полного окна.
        assert_eq!(segments[3].percent.round() as u32, 20);
    }

    #[test]
    fn context_map_segments_skip_provider_cache_categories() {
        let categories = vec![
            category("messages", 50),
            category("provider_cache_read", 40),
        ];
        let segments = context_map_segments(&categories, 50, None, None);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].label, "messages/history");
        // Без известного окна проценты — от суммы сегментов.
        assert_eq!(segments[0].percent.round() as u32, 100);
    }

    #[test]
    fn context_cache_status_tracks_cold_warming_and_hot_states() {
        assert_eq!(
            context_cache_status(100, 0, 0, Some(0)),
            ContextCacheStatus::Cold
        );
        assert_eq!(
            context_cache_status(100, 0, 80, Some(0)),
            ContextCacheStatus::Warming
        );
        assert_eq!(
            context_cache_status(100, 20, 0, Some(20)),
            ContextCacheStatus::Warming
        );
        assert_eq!(
            context_cache_status(100, 75, 0, Some(75)),
            ContextCacheStatus::Hot
        );
    }

    #[test]
    fn context_cache_view_model_handles_missing_usage() {
        let cache = context_cache_view_model(None);

        assert_eq!(cache.status, "n/a");
        assert_eq!(cache.input_tokens, "n/a");
        assert_eq!(cache.hit_rate, "n/a");
        assert_eq!(cache.hit_percent, 0);
    }

    #[test]
    fn context_cache_view_model_formats_provider_usage() {
        let usage = ContextUsageSnapshot {
            model_provider: "openai".to_owned(),
            model_name: "gpt-test".to_owned(),
            phase: Some("execute".to_owned()),
            estimated_input_tokens: 100,
            max_input_tokens: Some(1000),
            compaction_trigger_tokens: None,
            categories: Vec::new(),
            actual: Some(ContextActualUsage {
                input_tokens: 2000,
                output_tokens: 10,
                cached_input_tokens: Some(1500),
                cache_creation_input_tokens: Some(0),
                reasoning_output_tokens: None,
            }),
            source: "mixed".to_owned(),
            turn_id: None,
            timestamp_ms: None,
        };

        let cache = context_cache_view_model(Some(&usage));

        assert_eq!(cache.status, "hot");
        assert_eq!(cache.input_tokens, "2k");
        assert_eq!(cache.cached_input_tokens, "1.5k");
        assert_eq!(cache.cache_creation_input_tokens, "0");
        assert_eq!(cache.hit_rate, "75%");
        assert_eq!(cache.hit_percent, 75);
    }
}
