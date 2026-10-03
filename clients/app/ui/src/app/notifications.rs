//! Chats that finish or start waiting while the window is in the background
//! report through system notifications; a click opens that chat.
// Host builds compile the pure rules for tests only.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
use std::{collections::HashMap, path::PathBuf};

use crate::{session::summaries::sidebar_session_title, types::SessionSummary};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Attention {
    Finished,
    Approval,
    Input,
}

impl Attention {
    /// News between two activity statuses of one chat.
    pub(crate) fn between(previous: &str, next: &str) -> Option<Self> {
        if previous == next {
            return None;
        }
        match next {
            "idle" => Some(Self::Finished),
            "waiting_approval" => Some(Self::Approval),
            "waiting_input" => Some(Self::Input),
            _ => None,
        }
    }

    pub(crate) fn body(self) -> &'static str {
        match self {
            Self::Finished => "Агент закончил работу.",
            Self::Approval => "Агент ждёт подтверждения действия.",
            Self::Input => "Агент задал вопрос и ждёт ответа.",
        }
    }
}

/// Activity first seen after startup is a baseline, not news.
pub(crate) fn attention_changes(
    seen: &mut HashMap<PathBuf, &'static str>,
    sessions: &[SessionSummary],
) -> Vec<(String, Attention, String)> {
    let mut changes = Vec::new();
    for session in sessions {
        let Some(activity) = &session.activity else {
            continue;
        };
        let status = activity.status.as_str();
        if let Some(previous) = seen.insert(session.session_dir.clone(), status)
            && let Some(attention) = Attention::between(previous, status)
        {
            changes.push((
                sidebar_session_title(session),
                attention,
                session.session_dir.display().to_string(),
            ));
        }
    }
    changes
}

#[cfg(target_arch = "wasm32")]
pub(super) fn install(
    sessions: leptos::prelude::ReadSignal<Vec<SessionSummary>>,
    enabled: leptos::prelude::RwSignal<bool>,
    open: impl Fn(SessionSummary) + Copy + 'static,
) {
    use leptos::prelude::*;
    use wasm_bindgen::{JsCast, prelude::*};

    #[wasm_bindgen(raw_module = "/ui/notify.js")]
    extern "C" {
        #[wasm_bindgen(js_name = showNotification)]
        fn show_notification(title: &str, body: &str, session_dir: &str);
        #[wasm_bindgen(js_name = listenNotificationClicks)]
        fn listen_notification_clicks() -> js_sys::Function;
    }

    let seen = StoredValue::new_local(HashMap::new());
    Effect::new(move |_| {
        let changes = sessions.with(|items| {
            let mut changes = Vec::new();
            seen.update_value(|seen| changes = attention_changes(seen, items));
            changes
        });
        if enabled.get_untracked() {
            for (title, attention, session_dir) in changes {
                show_notification(&title, attention.body(), &session_dir);
            }
        }
    });
    let clicks = window_event_listener_untyped("proteus-open-session", move |event| {
        let Some(session_dir) = event
            .dyn_ref::<web_sys::CustomEvent>()
            .and_then(|event| event.detail().as_string())
        else {
            return;
        };
        let session = sessions.with_untracked(|items| {
            items
                .iter()
                .find(|session| session.session_dir.display().to_string() == session_dir)
                .cloned()
        });
        if let Some(session) = session {
            open(session);
        }
    });
    let stop = StoredValue::new_local(listen_notification_clicks());
    on_cleanup(move || {
        clicks.remove();
        stop.with_value(|stop| {
            let _ = stop.call0(&JsValue::NULL);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SessionActivityInfo;
    use proteus_contracts::app_protocol::AppSessionActivityStatus as SessionActivityStatus;

    fn session(dir: &str, status: Option<SessionActivityStatus>) -> SessionSummary {
        SessionSummary {
            session_dir: dir.into(),
            session_id: "12345678-0000-0000-0000-000000000000".parse().unwrap(),
            workspace_path: "/tmp/workspace".into(),
            message_count: 1,
            updated_at_ms: None,
            preview: Some("Починить сборку".into()),
            activity: status.map(|status| SessionActivityInfo {
                status,
                running_runs: 0,
                running_run_ids: Vec::new(),
                pending_approvals: 0,
                pending_user_inputs: 0,
            }),
        }
    }

    #[test]
    fn only_changes_after_the_first_sight_are_news() {
        use SessionActivityStatus::*;
        let mut seen = HashMap::new();
        assert!(attention_changes(&mut seen, &[session("/a", Some(Running))]).is_empty());
        assert!(attention_changes(&mut seen, &[session("/a", Some(Running))]).is_empty());
        assert!(
            attention_changes(&mut seen, &[session("/b", Some(Idle)), session("/c", None)])
                .is_empty()
        );
        let changes = attention_changes(
            &mut seen,
            &[
                session("/a", Some(WaitingApproval)),
                session("/b", Some(Running)),
            ],
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].1, Attention::Approval);
        assert_eq!(changes[0].2, "/a");
        let changes = attention_changes(
            &mut seen,
            &[session("/a", Some(Idle)), session("/b", Some(WaitingInput))],
        );
        assert_eq!(
            changes.iter().map(|change| change.1).collect::<Vec<_>>(),
            [Attention::Finished, Attention::Input]
        );
    }
}
