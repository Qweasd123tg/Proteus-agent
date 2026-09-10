use super::*;
fn session_summary(preview: Option<&str>, message_count: usize) -> SessionSummary {
    SessionSummary {
        session_dir: "/tmp/session".to_owned(),
        session_id: "1234567890".to_owned(),
        workspace_path: "/tmp/workspace".to_owned(),
        message_count,
        updated_at_ms: None,
        preview: preview.map(ToOwned::to_owned),
        activity: None,
    }
}

#[test]
fn sidebar_empty_session_uses_new_chat_without_preview_placeholder() {
    let session = session_summary(None, 0);

    assert_eq!(sidebar_session_title(&session), "Новый чат");
    assert_eq!(sidebar_session_preview(&session), None);
}

#[test]
fn sidebar_session_render_key_changes_when_activity_changes() {
    let mut session = session_summary(Some("work"), 1);
    let idle_key = sidebar_session_render_key(&session);

    session.activity = Some(SessionActivityInfo {
        status: "running".to_owned(),
        running_runs: 1,
        running_run_ids: vec!["run-1".to_owned()],
        pending_approvals: 0,
        pending_user_inputs: 0,
    });

    assert_ne!(sidebar_session_render_key(&session), idle_key);
}

#[test]
fn active_session_activity_restores_running_run_state() {
    let activity = SessionActivityInfo {
        status: "running".to_owned(),
        running_runs: 1,
        running_run_ids: vec!["run-1".to_owned()],
        pending_approvals: 0,
        pending_user_inputs: 0,
    };

    assert_eq!(
        active_session_activity_state(Some(&activity)),
        ActiveSessionActivityState {
            is_sending: true,
            active_run_id: Some("run-1".to_owned()),
            agent_status: "работает".to_owned(),
        }
    );
}

#[test]
fn active_session_activity_idle_clears_run_state() {
    let activity = SessionActivityInfo {
        status: "idle".to_owned(),
        running_runs: 0,
        running_run_ids: Vec::new(),
        pending_approvals: 0,
        pending_user_inputs: 0,
    };

    assert_eq!(
        active_session_activity_state(Some(&activity)),
        ActiveSessionActivityState {
            is_sending: false,
            active_run_id: None,
            agent_status: "ожидает".to_owned(),
        }
    );
}
