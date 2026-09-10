use crate::{
    api::get_json,
    types::*,
    ui_utils::{compact_text, compact_title},
};
use leptos::{prelude::*, task::spawn_local};

pub(crate) fn sidebar_session_title(session: &SessionSummary) -> String {
    if let Some(preview) = session
        .preview
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        compact_title(preview)
    } else if session.message_count == 0 {
        "Новый чат".to_owned()
    } else {
        "Сессия".to_owned()
    }
}

pub(crate) fn sidebar_session_preview(session: &SessionSummary) -> Option<String> {
    session
        .preview
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .map(|text| compact_text(text, 80))
}

pub(crate) fn sidebar_session_activity_label(
    activity: Option<&SessionActivityInfo>,
) -> Option<String> {
    let activity = activity?;
    match activity.status.as_str() {
        "waiting_input" => Some("ждёт ответ".to_owned()),
        "waiting_approval" => Some("ждёт доступ".to_owned()),
        "running" => Some("работает".to_owned()),
        "idle" => None,
        other if !other.trim().is_empty() => Some(other.replace('_', " ")),
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveSessionActivityState {
    pub(crate) is_sending: bool,
    pub(crate) active_run_id: Option<String>,
    pub(crate) agent_status: String,
}

pub(crate) fn active_session_activity_state(
    activity: Option<&SessionActivityInfo>,
) -> ActiveSessionActivityState {
    let is_sending = activity.is_some_and(session_activity_is_busy);
    let active_run_id = activity
        .and_then(|activity| activity.running_run_ids.first())
        .cloned();
    let agent_status = match activity.map(|activity| activity.status.as_str()) {
        Some("waiting_input") => "ждёт ответ",
        Some("waiting_approval") => "ждёт доступ",
        Some("running") => "работает",
        Some("idle") | None => "ожидает",
        Some(other) if !other.trim().is_empty() => other,
        Some(_) => "ожидает",
    }
    .replace('_', " ");

    ActiveSessionActivityState {
        is_sending,
        active_run_id,
        agent_status,
    }
}

pub(crate) fn apply_active_session_activity(
    activity: Option<&SessionActivityInfo>,
    set_is_sending: WriteSignal<bool>,
    set_active_run_id: WriteSignal<Option<String>>,
    set_agent_status: WriteSignal<String>,
) {
    let state = active_session_activity_state(activity);
    set_is_sending.set(state.is_sending);
    set_active_run_id.set(state.active_run_id);
    set_agent_status.set(state.agent_status);
}

pub(super) fn session_activity_is_busy(activity: &SessionActivityInfo) -> bool {
    activity.running_runs > 0
        || activity.pending_approvals > 0
        || activity.pending_user_inputs > 0
        || matches!(
            activity.status.as_str(),
            "running" | "waiting_approval" | "waiting_input"
        )
}

pub(crate) fn sidebar_session_activity_dot_class(
    activity: Option<&SessionActivityInfo>,
) -> &'static str {
    match activity.map(|activity| activity.status.as_str()) {
        Some("waiting_input" | "waiting_approval") => "session-status-dot warning",
        Some("running") => "session-status-dot running",
        Some("idle") | None => "session-status-dot",
        Some(_) => "session-status-dot running",
    }
}

pub(crate) fn sidebar_session_render_key(session: &SessionSummary) -> String {
    let activity = session.activity.as_ref();
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        session.session_dir,
        session.message_count,
        session.updated_at_ms.unwrap_or_default(),
        session.preview.as_deref().unwrap_or_default(),
        activity
            .map(|activity| activity.status.as_str())
            .unwrap_or(""),
        activity.map(|activity| activity.running_runs).unwrap_or(0),
        activity
            .map(|activity| activity.running_run_ids.join(","))
            .unwrap_or_default(),
        activity
            .map(|activity| activity.pending_approvals + activity.pending_user_inputs)
            .unwrap_or(0),
    )
}

pub(crate) fn load_sidebar_sessions(
    set_sessions: WriteSignal<Vec<SessionSummary>>,
    set_status: WriteSignal<String>,
) {
    set_status.set("загружаю сессии".to_owned());
    spawn_local(async move {
        match get_json::<Vec<SessionSummary>>("/sessions/current").await {
            Ok(items) => {
                let count = items.len();
                set_sessions.set(items);
                set_status.set(if count == 0 {
                    "прошлых сессий нет".to_owned()
                } else {
                    format!("{count} сессий")
                });
            }
            Err(error) => {
                set_sessions.set(Vec::new());
                set_status.set(format!("сессии недоступны: {error}"));
            }
        }
    });
}

#[cfg(test)]
mod tests;
