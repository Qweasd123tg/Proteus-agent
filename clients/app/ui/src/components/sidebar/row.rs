use super::preferences::Preferences;
use crate::{
    components::icons::*, session::summaries::*, types::SessionSummary,
    ui_utils::relative_time_from_now,
};
use leptos::prelude::*;

#[component]
pub(super) fn SessionRow<O, D>(
    session: SessionSummary,
    preferences: Preferences,
    active_session_dir: ReadSignal<Option<String>>,
    on_open: O,
    on_delete: D,
) -> impl IntoView
where
    O: Fn(SessionSummary) + Copy + Send + 'static,
    D: Fn(SessionSummary) + Copy + Send + 'static,
{
    let id = StoredValue::new(session.session_dir.to_string_lossy().into_owned());
    let original = StoredValue::new(sidebar_session_title(&session));
    let title = move || {
        preferences
            .entry(&id.get_value())
            .title
            .unwrap_or_else(|| original.get_value())
    };
    let detail = format!(
        "{} сообщений · {}\n{}\n{}",
        session.message_count,
        relative_time_from_now(session.updated_at_ms),
        sidebar_session_activity_label(session.activity.as_ref()).unwrap_or_default(),
        session.workspace_path.display()
    );
    let dot = sidebar_session_activity_dot_class(session.activity.as_ref());
    let for_open = session.clone();
    let for_delete = session;
    view! {
        <li class="session-list-item">
            <div class="session-item-shell" data-session-dir=id.get_value() data-hover-title=title data-hover-detail=detail
                data-pinned=move || preferences.entry(&id.get_value()).pinned.to_string()
                data-archived=move || preferences.entry(&id.get_value()).archived.to_string()>
                <button type="button" class="session-item session-history-item" class:active=move || active_session_dir.get().as_deref()==Some(id.get_value().as_str())
                    on:click=move |_|on_open(for_open.clone())>
                    <div class="session-item-header"><span class="session-title-line"><span class=dot></span><span class="session-id">{title}</span>
                        <Show when=move || preferences.entry(&id.get_value()).pinned><PinIcon/></Show>
                    </span></div>
                </button>
                <button type="button" class="session-more" data-sidebar-menu="" aria-label="Действия с чатом" title="Действия с чатом"><MoreIcon/></button>
                <button type="button" data-delete-session="" hidden on:click=move |_|on_delete(for_delete.clone())>"Удалить чат"</button>
            </div>
        </li>
    }
}
