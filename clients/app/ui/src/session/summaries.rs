use crate::{
    types::*,
    ui_utils::{compact_text, compact_title},
};
use leptos::prelude::*;
use proteus_contracts::app_protocol::AppSessionActivityStatus;

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
    let status = activity_agent_status(activity?.status);
    (status != AgentStatus::Idle).then(|| status.label())
}

fn activity_agent_status(status: AppSessionActivityStatus) -> AgentStatus {
    match status {
        AppSessionActivityStatus::WaitingInput => AgentStatus::WaitingAnswer,
        AppSessionActivityStatus::WaitingApproval => AgentStatus::WaitingApproval { subagent: false },
        AppSessionActivityStatus::Running => AgentStatus::Running,
        AppSessionActivityStatus::Idle => AgentStatus::Idle,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveSessionActivityState {
    pub(crate) is_sending: bool,
    pub(crate) active_run_id: Option<String>,
    pub(crate) agent_status: AgentStatus,
}

pub(crate) fn active_session_activity_state(
    activity: Option<&SessionActivityInfo>,
) -> ActiveSessionActivityState {
    let is_sending = activity.is_some_and(session_activity_is_busy);
    let active_run_id = activity
        .and_then(|activity| activity.running_run_ids.first())
        .cloned();
    ActiveSessionActivityState {
        is_sending,
        active_run_id,
        agent_status: activity.map_or(AgentStatus::Idle, |activity| activity_agent_status(activity.status)),
    }
}

pub(crate) fn apply_active_session_activity(
    activity: Option<&SessionActivityInfo>,
    set_is_sending: WriteSignal<bool>,
    set_active_run_id: WriteSignal<Option<String>>,
    set_agent_status: WriteSignal<AgentStatus>,
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
        || activity.status != AppSessionActivityStatus::Idle
}

pub(crate) fn sidebar_session_activity_dot_class(
    activity: Option<&SessionActivityInfo>,
) -> &'static str {
    match activity.map(|activity| activity.status) {
        Some(AppSessionActivityStatus::WaitingInput | AppSessionActivityStatus::WaitingApproval) => {
            "session-status-dot warning"
        }
        Some(AppSessionActivityStatus::Running) => "session-status-dot running",
        Some(AppSessionActivityStatus::Idle) | None => "session-status-dot",
    }
}

pub(crate) fn sidebar_session_render_key(session: &SessionSummary) -> String {
    let activity = session.activity.as_ref();
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        session.session_dir.display(),
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

#[cfg(test)]
mod tests;
