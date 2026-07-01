use leptos::prelude::*;
use web_sys::MouseEvent;

use crate::app_helpers::{
    sidebar_session_activity_dot_class, sidebar_session_activity_label, sidebar_session_preview,
    sidebar_session_render_key, sidebar_session_title,
};
use crate::types::*;
use crate::ui_utils::{relative_time_from_now, short_id};

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn SidebarView<R, N, T, B, O, D, RS, AC>(
    sidebar_width: ReadSignal<i32>,
    sidebar_collapsed: ReadSignal<bool>,
    workspace_label: ReadSignal<String>,
    sidebar_sessions: ReadSignal<Vec<SessionSummary>>,
    sidebar_sessions_status: ReadSignal<String>,
    active_session_dir: ReadSignal<Option<String>>,
    runtime_state: RS,
    activity: AC,
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
    RS: Fn() -> String + Copy + Send + 'static,
    AC: Fn() -> Vec<(&'static str, String)> + Copy + Send + 'static,
{
    view! {
        <aside class="sidebar" style=move || format!("width: {}px", sidebar_width.get())>
            <div class="sidebar-header">
                <h2>
                    "Proteus"
                    <span>"web"</span>
                </h2>
                <div class="sidebar-header-actions">
                    <button type="button" title="Обновить сессии" on:click=on_refresh>
                        "↻"
                    </button>
                    <button type="button" title="Новая сессия" on:click=on_new_session>
                        "+"
                    </button>
                    <button
                        type="button"
                        title=move || if sidebar_collapsed.get() {
                            "Развернуть меню"
                        } else {
                            "Свернуть меню"
                        }
                        on:click=on_toggle
                    >
                        {move || if sidebar_collapsed.get() { "›" } else { "‹" }}
                    </button>
                </div>
            </div>
            <div
                class="sidebar-resize-handle"
                aria-hidden="true"
                on:mousedown=on_begin_resize
            ></div>

            <div class="sidebar-search">
                <input
                    type="text"
                    placeholder=move || {
                        let workspace = workspace_label.get();
                        if workspace == "waiting for session" {
                            sidebar_sessions_status.get()
                        } else {
                            let count = sidebar_sessions.with(|sessions| {
                                sessions
                                    .iter()
                                    .filter(|session| {
                                        session.workspace_path.as_deref()
                                            == Some(workspace.as_str())
                                    })
                                    .count()
                            });
                            format!("{count} сессий в текущей папке")
                        }
                    }
                    readonly=true
                />
            </div>

            <div class="sessions-list">
                <ul class="session-list">
                    <For
                        each=move || {
                            let workspace = workspace_label.get();
                            sidebar_sessions.with(|sessions| {
                                sessions
                                    .iter()
                                    .filter(|session| {
                                        workspace != "waiting for session"
                                            && session.workspace_path.as_deref()
                                                == Some(workspace.as_str())
                                    })
                                    .cloned()
                                    .collect::<Vec<_>>()
                            })
                        }
                        key=|session| sidebar_session_render_key(session)
                        children=move |session| {
                            let workspace = session
                                .workspace_path
                                .clone()
                                .unwrap_or_else(|| "неизвестный workspace".to_owned());
                            let session_id = session
                                .session_id
                                .as_deref()
                                .map(short_id)
                                .unwrap_or("legacy")
                                .to_owned();
                            let title = sidebar_session_title(&session);
                            let preview = sidebar_session_preview(&session);
                            let activity_label =
                                sidebar_session_activity_label(session.activity.as_ref());
                            let activity_dot_class =
                                sidebar_session_activity_dot_class(session.activity.as_ref());
                            let message_count = session.message_count;
                            let updated_at = relative_time_from_now(session.updated_at_ms);
                            let resumable = session.resumable;
                            let active_session_dir_value = session.session_dir.clone();
                            let session_for_click = session.clone();
                            let session_for_delete = session.clone();
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
                                            disabled=!resumable
                                            title=workspace.clone()
                                            on:click=move |_| on_open_session(session_for_click.clone())
                                        >
                                            <div class="session-item-header">
                                                <span class="session-title-line">
                                                    <span class=activity_dot_class></span>
                                                    <span class="session-id">{title}</span>
                                                </span>
                                                <code class="session-code">{session_id}</code>
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
                                                <span class="session-time">{format!("{message_count} сообщений")}</span>
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

            <section class="sidebar-panel">
                <div class="runtime-summary">
                    <span class="panel-kicker">"Runtime"</span>
                    <strong>{runtime_state}</strong>
                    <code title=move || workspace_label.get()>{move || workspace_label.get()}</code>
                </div>
                <div class="activity-grid">
                    <For
                        each=activity
                        key=|item| item.0
                        children=move |(label, value)| {
                            view! {
                                <div class="activity-row">
                                    <span>{label}</span>
                                    <strong>{value}</strong>
                                </div>
                            }
                        }
                    />
                </div>
            </section>
        </aside>
    }
}
