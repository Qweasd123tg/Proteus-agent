use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::SessionId;

/// Canonical summary одной session для app-server clients.
///
/// Durable и live sessions используют один DTO: storage заполняет общие
/// поля, а control-plane при необходимости добавляет текущую activity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AppSessionSummary {
    pub session_dir: PathBuf,
    pub session_id: SessionId,
    pub workspace_path: PathBuf,
    pub message_count: usize,
    pub updated_at_ms: Option<u64>,
    pub preview: Option<String>,
    pub activity: Option<AppSessionActivity>,
}

impl AppSessionSummary {
    pub fn new(
        session_dir: PathBuf,
        session_id: SessionId,
        workspace_path: PathBuf,
        message_count: usize,
        updated_at_ms: Option<u64>,
        preview: Option<String>,
    ) -> Self {
        Self {
            session_dir,
            session_id,
            workspace_path,
            message_count,
            updated_at_ms,
            preview,
            activity: None,
        }
    }

    pub fn with_activity(mut self, activity: AppSessionActivity) -> Self {
        self.activity = Some(activity);
        self
    }
}

/// Короткий UI/control-plane snapshot работы одной session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppSessionActivity {
    pub status: AppSessionActivityStatus,
    pub running_turns: usize,
    pub running_turn_ids: Vec<String>,
    pub pending_user_inputs: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AppSessionActivityStatus {
    Idle,
    Running,
    WaitingInput,
}

impl AppSessionActivityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::WaitingInput => "waiting_input",
        }
    }
}

impl AppSessionActivity {
    pub fn from_counts(running_turns: usize, pending_user_inputs: usize) -> Self {
        let status = if pending_user_inputs > 0 {
            AppSessionActivityStatus::WaitingInput
        } else if running_turns > 0 {
            AppSessionActivityStatus::Running
        } else {
            AppSessionActivityStatus::Idle
        };
        Self {
            status,
            running_turns,
            running_turn_ids: Vec::new(),
            pending_user_inputs,
        }
    }

    pub fn from_running_turn_ids(
        running_turn_ids: Vec<String>,
        pending_user_inputs: usize,
    ) -> Self {
        let mut activity = Self::from_counts(running_turn_ids.len(), pending_user_inputs);
        activity.running_turn_ids = running_turn_ids;
        activity
    }

    pub fn is_idle(&self) -> bool {
        self.status == AppSessionActivityStatus::Idle
            && self.running_turns == 0
            && self.running_turn_ids.is_empty()
            && self.pending_user_inputs == 0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::new_session_id;

    #[test]
    fn session_summary_roundtrips_full_wire_shape() {
        let summary = AppSessionSummary::new(
            PathBuf::from("/tmp/session-1"),
            new_session_id(),
            PathBuf::from("/workspace"),
            3,
            Some(42),
            Some("first message".to_owned()),
        )
        .with_activity(AppSessionActivity::from_counts(1, 0));

        let value = serde_json::to_value(&summary).expect("summary JSON");
        assert_eq!(value["activity"]["status"], "running");

        let decoded: AppSessionSummary = serde_json::from_value(value).expect("summary decode");
        assert_eq!(decoded, summary);
    }

    #[test]
    fn session_summary_rejects_unknown_wire_fields() {
        serde_json::from_value::<AppSessionSummary>(json!({
            "session_dir": "/tmp/session-1",
            "session_id": new_session_id(),
            "workspace_path": "/workspace",
            "message_count": 0,
            "updated_at_ms": null,
            "preview": null,
            "activity": null,
            "legacy_resumable": true,
        }))
        .expect_err("unknown summary fields must fail");
    }
}
