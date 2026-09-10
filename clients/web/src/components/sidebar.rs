use leptos::prelude::*;
use web_sys::MouseEvent;

mod header;
use crate::session::summaries::{
    sidebar_session_activity_dot_class, sidebar_session_activity_label, sidebar_session_preview,
    sidebar_session_render_key, sidebar_session_title,
};
use crate::types::*;
use crate::ui_utils::relative_time_from_now;
use header::SidebarHeader;

/// Сколько сессий помещается в рейку свёрнутого сайдбара.
const SIDEBAR_RAIL_LIMIT: usize = 10;

/// Класс индикатора сессии в свёрнутой рейке: спиннер у работающих,
/// «?» у ждущих человека, точка у остальных.
fn rail_session_class(session: &SessionSummary) -> &'static str {
    match session.activity.as_ref().map(|a| a.status.as_str()) {
        Some("waiting_input" | "waiting_approval") => "sidebar-rail-session waiting",
        Some("running") => "sidebar-rail-session running",
        _ => "sidebar-rail-session",
    }
}

fn rail_sessions(workspace: &str, sessions: &[SessionSummary]) -> Vec<SessionSummary> {
    if workspace == "waiting for session" {
        return Vec::new();
    }
    sessions
        .iter()
        .filter(|session| session.workspace_path == workspace)
        .take(SIDEBAR_RAIL_LIMIT)
        .cloned()
        .collect()
}

fn rail_sessions_total(workspace: &str, sessions: &[SessionSummary]) -> usize {
    if workspace == "waiting for session" {
        return 0;
    }
    sessions
        .iter()
        .filter(|session| session.workspace_path == workspace)
        .count()
}

/// Фильтр списка сессий по строке поиска: заголовок, превью и путь сессии.
fn session_matches_query(session: &SessionSummary, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    sidebar_session_title(session)
        .to_lowercase()
        .contains(&query)
        || sidebar_session_preview(session)
            .unwrap_or_default()
            .to_lowercase()
            .contains(&query)
        || session.session_dir.to_lowercase().contains(&query)
}

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn SidebarView<R, N, T, B, O, D>(
    sidebar_width: ReadSignal<i32>,
    sidebar_collapsed: ReadSignal<bool>,
    workspace_label: ReadSignal<String>,
    sidebar_sessions: ReadSignal<Vec<SessionSummary>>,
    sidebar_sessions_status: ReadSignal<String>,
    active_session_dir: ReadSignal<Option<String>>,
    on_refresh: R,
    on_new_session: N,
    on_toggle: T,
    on_begin_resize: B,
    on_open_session: O,
    on_delete_session: D,
) -> impl IntoView
where
    R: Fn(MouseEvent) + Copy + 'static,
    N: Fn(MouseEvent) + Copy + 'static,
    T: Fn(MouseEvent) + Copy + 'static,
    B: Fn(MouseEvent) + Copy + 'static,
    O: Fn(SessionSummary) + Copy + Send + 'static,
    D: Fn(SessionSummary) + Copy + Send + 'static,
{
    let (query, set_query) = signal(String::new());
    view! {
        // Состояние рейки задаёт CSS; выбранная ширина сохраняется для раскрытия.
        <aside class="sidebar" style=move || format!("--sidebar-width: {}px", sidebar_width.get())>
            <div class="sidebar-surface" inert=move || sidebar_collapsed.get().then_some("")>
            <SidebarHeader collapsed=sidebar_collapsed show_title=true on_toggle on_refresh on_new_session />
            <div class="sidebar-search">
                <input
                    type="text"
                    aria-label="Найти чат"
                    placeholder=move || {
                        let workspace = workspace_label.get();
                        if workspace == "waiting for session" {
                            sidebar_sessions_status.get()
                        } else {
                            "Найти чат".to_owned()
                        }
                    }
                    prop:value=move || query.get()
                    on:input:target=move |ev| set_query.set(ev.target().value())
                />
            </div>

            <div class="sessions-list">
                <ul class="session-list">
                    <For
                        each=move || {
                            let workspace = workspace_label.get();
                            let query = query.get();
                            sidebar_sessions.with(|sessions| {
                                sessions
                                    .iter()
                                    .filter(|session| {
                                        workspace != "waiting for session"
                                            && session.workspace_path == workspace
                                            && session_matches_query(session, &query)
                                    })
                                    .cloned()
                                    .collect::<Vec<_>>()
                            })
                        }
                        key=|session| sidebar_session_render_key(session)
                        children=move |session| {
                            let workspace = session.workspace_path.clone();
                            let title = sidebar_session_title(&session);
                            let preview = sidebar_session_preview(&session)
                                .filter(|preview| preview.trim() != title.trim());
                            let activity_label =
                                sidebar_session_activity_label(session.activity.as_ref());
                            let activity_dot_class =
                                sidebar_session_activity_dot_class(session.activity.as_ref());
                            let message_count = session.message_count;
                            let updated_at = relative_time_from_now(session.updated_at_ms);
                            let active_session_dir_value = session.session_dir.clone();
                            let session_for_click = session.clone();
                            let session_for_delete = session.clone();
                            let tooltip = format!("{title}\n{workspace}\n{message_count} сообщений");
                            view! {
                                <li class="session-list-item">
                                    <div class="session-item-shell">
                                        <button
                                            type="button"
                                            class="session-item session-history-item"
                                            class:active=move || {
                                                active_session_dir.get().as_deref()
                                                    == Some(active_session_dir_value.as_str())
                                            }
                                            title=tooltip
                                            on:click=move |_| on_open_session(session_for_click.clone())
                                        >
                                            <div class="session-item-header">
                                                <span class="session-title-line">
                                                    <span class=activity_dot_class></span>
                                                    <span class="session-id">{title}</span>
                                                </span>
                                            </div>
                                            {match preview {
                                                Some(preview) => view! {
                                                    <div class="session-preview">{preview}</div>
                                                }.into_any(),
                                                None => ().into_any(),
                                            }}
                                            <div class="session-meta">
                                                {match activity_label {
                                                    Some(label) => view! {
                                                        <span class="session-time session-activity">{label}</span>
                                                    }.into_any(),
                                                    None => ().into_any(),
                                                }}
                                                <span class="session-time">{updated_at}</span>
                                            </div>
                                        </button>
                                        <button
                                            type="button"
                                            class="session-delete"
                                            title="Удалить чат"
                                            aria-label="Удалить чат"
                                            on:click=move |_| on_delete_session(session_for_delete.clone())
                                        >
                                            "×"
                                        </button>
                                    </div>
                                </li>
                            }
                        }
                    />
                </ul>
            </div>

            </div>
            <div class="sidebar-rail-surface" inert=move || (!sidebar_collapsed.get()).then_some("")>
            <SidebarHeader collapsed=sidebar_collapsed on_toggle on_refresh on_new_session />
            // Рейка свёрнутого сайдбара: сессии workspace индикаторами —
            // спиннер у работающих, «?» у ждущих ответа, точка у остальных;
            // при наведении — поповер с деталями (единый стиль .rail-popover).
            // Кэп, чтобы колонка не переполнялась: рейка не скроллится,
            // иначе поповеры обрезаются.
            <div class="sidebar-rail">
                <For
                    each=move || {
                        rail_sessions(
                            &workspace_label.get(),
                            &sidebar_sessions.get(),
                        )
                    }
                    key=|session| sidebar_session_render_key(session)
                    children=move |session| {
                        let class = rail_session_class(&session);
                        let waiting = class.ends_with("waiting");
                        let title = sidebar_session_title(&session);
                        let status_label =
                            sidebar_session_activity_label(session.activity.as_ref())
                                .unwrap_or_else(|| "ожидает".to_owned());
                        let aria = format!("{title} · {status_label}");
                        let message_count = session.message_count;
                        let updated_at = relative_time_from_now(session.updated_at_ms);
                        let session_dir = session.session_dir.clone();
                        let session_for_click = session.clone();
                        view! {
                            <div class="sidebar-rail-item rail-popover-host">
                                <button
                                    type="button"
                                    class=class
                                    class:active=move || {
                                        active_session_dir.get().as_deref()
                                            == Some(session_dir.as_str())
                                    }
                                    aria-label=aria
                                    on:click=move |_| on_open_session(session_for_click.clone())
                                >
                                    {waiting.then_some("?")}
                                </button>
                                <div class="rail-popover rail-popover-right">
                                    <div class="rail-popover-head">
                                        <span class="panel-kicker">"Сессия"</span>
                                        <code>{status_label}</code>
                                    </div>
                                    <div class="rail-popover-title">{title}</div>
                                    <div class="info-row">
                                        <span>"Сообщений"</span>
                                        <code>{message_count.to_string()}</code>
                                    </div>
                                    <div class="info-row">
                                        <span>"Обновлена"</span>
                                        <code>{updated_at}</code>
                                    </div>
                                </div>
                            </div>
                        }
                    }
                />
                {move || {
                    let total = rail_sessions_total(
                        &workspace_label.get(),
                        &sidebar_sessions.get(),
                    );
                    if total > SIDEBAR_RAIL_LIMIT {
                        view! {
                            <div class="sidebar-rail-more">
                                {format!("+{}", total - SIDEBAR_RAIL_LIMIT)}
                            </div>
                        }.into_any()
                    } else {
                        ().into_any()
                    }
                }}
            </div>

            </div>
            <div
                class="sidebar-resize-handle"
                aria-hidden="true"
                on:mousedown=on_begin_resize
            ></div>

        </aside>
    }
}
