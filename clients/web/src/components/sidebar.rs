use leptos::prelude::*;
use web_sys::MouseEvent;

#[cfg(target_arch = "wasm32")]
mod browser;
mod footer;
mod header;
mod preferences;
mod row;
use crate::session::summaries::{
    sidebar_session_activity_label, sidebar_session_preview, sidebar_session_render_key,
    sidebar_session_title,
};
use crate::types::*;
use crate::ui_utils::relative_time_from_now;
pub(crate) use footer::SidebarFooter;
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
        .filter(|session| session.workspace_path == std::path::Path::new(&workspace))
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
        .filter(|session| session.workspace_path == std::path::Path::new(&workspace))
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
        || session
            .session_dir
            .to_string_lossy()
            .to_lowercase()
            .contains(&query)
}

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn SidebarView<R, N, B, O, D>(
    sidebar_width: ReadSignal<i32>,
    sidebar_collapsed: ReadSignal<bool>,
    workspace_label: ReadSignal<String>,
    sidebar_sessions: ReadSignal<Vec<SessionSummary>>,
    sidebar_sessions_status: ReadSignal<String>,
    active_session_dir: ReadSignal<Option<String>>,
    on_refresh: R,
    on_new_session: N,
    on_begin_resize: B,
    on_open_session: O,
    on_delete_session: D,
    children: Children,
) -> impl IntoView
where
    R: Fn(MouseEvent) + Copy + 'static,
    N: Fn(MouseEvent) + Copy + 'static,
    B: Fn(MouseEvent) + Copy + 'static,
    O: Fn(SessionSummary) + Copy + Send + 'static,
    D: Fn(SessionSummary) + Copy + Send + 'static,
{
    let (query, set_query) = signal(String::new());
    let preferences = preferences::Preferences::new();
    let root = NodeRef::<leptos::html::Aside>::new();
    #[cfg(target_arch = "wasm32")]
    browser::attach(root, preferences);
    let visible_sessions = move || {
        let workspace = workspace_label.get();
        let query = query.get();
        let archived = preferences.archived.get();
        let mut sessions = sidebar_sessions.with(|items| {
            items
                .iter()
                .filter(|session| {
                    let entry = preferences.entry(&session.session_dir.to_string_lossy());
                    workspace != "waiting for session"
                        && session.workspace_path == std::path::Path::new(&workspace)
                        && entry.archived == archived
                        && (session_matches_query(session, &query)
                            || entry.title.is_some_and(|title| {
                                title.to_lowercase().contains(&query.trim().to_lowercase())
                            }))
                })
                .cloned()
                .collect::<Vec<_>>()
        });
        sessions.sort_by_key(|session| {
            !preferences
                .entry(&session.session_dir.to_string_lossy())
                .pinned
        });
        sessions
    };
    view! {
        // Состояние рейки задаёт CSS; выбранная ширина сохраняется для раскрытия.
        <aside class="sidebar" node_ref=root data-show-archived=move || preferences.archived.get().to_string() style=move || format!("--sidebar-width: {}px", sidebar_width.get())>
            <div class="sidebar-surface" inert=move || sidebar_collapsed.get().then_some("")>
            <SidebarHeader on_refresh on_new_session />
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

            <div class="sidebar-project" data-workspace=move || workspace_label.get() data-hover-title=move || crate::ui_utils::short_path(&workspace_label.get())
                data-hover-detail=move || format!("{} чатов\n{}",rail_sessions_total(&workspace_label.get(),&sidebar_sessions.get()),workspace_label.get())>
                <super::icons::FolderIcon />
                <span>{move || crate::ui_utils::short_path(&workspace_label.get())}</span>
                <button type="button" class="project-more" data-sidebar-menu="" aria-label="Действия с проектом"><super::icons::MoreIcon/></button>
                <button type="button" class="project-new" title="Новый чат в проекте" aria-label="Новый чат в проекте" on:click=on_new_session><super::icons::EditIcon/></button>
            </div>
            <Show when=move || preferences.archived.get()><button class="sidebar-archive-back" on:click=move |_|preferences.archived.set(false)>"← Архив · вернуться к чатам"</button></Show>
            <p class="sidebar-preferences-error" role="status">{move || preferences.error.get()}</p>
            <div class="sessions-list">
                <ul class="session-list">
                    <For
                        each=visible_sessions
                        key=|session| sidebar_session_render_key(session)
                        children=move |session| view! { <row::SessionRow session preferences active_session_dir on_open=on_open_session on_delete=on_delete_session/> }
                    />
                </ul>
            </div>

            </div>
            <div class="sidebar-rail-surface" inert=move || (!sidebar_collapsed.get()).then_some("")>
            <SidebarHeader on_refresh on_new_session />
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
                            &visible_sessions(),
                        )
                    }
                    key=|session| sidebar_session_render_key(session)
                    children=move |session| {
                        let class = rail_session_class(&session);
                        let waiting = class.ends_with("waiting");
                        let original_title = StoredValue::new(sidebar_session_title(&session));
                        let title_id = StoredValue::new(session.session_dir.to_string_lossy().into_owned());
                        let title = move || preferences.entry(&title_id.get_value()).title.unwrap_or_else(|| original_title.get_value());
                        let status_label =
                            sidebar_session_activity_label(session.activity.as_ref())
                                .unwrap_or_else(|| "ожидает".to_owned());
                        let aria_status = StoredValue::new(status_label.clone());
                        let aria = move || format!("{} · {}", title(), aria_status.get_value());
                        let message_count = session.message_count;
                        let updated_at = relative_time_from_now(session.updated_at_ms);
                        let session_dir = session.session_dir.to_string_lossy().into_owned();
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
                        &visible_sessions(),
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
            {children()}
            <div
                class="sidebar-resize-handle"
                aria-hidden="true"
                on:mousedown=on_begin_resize
            ></div>

        </aside>
    }
}
