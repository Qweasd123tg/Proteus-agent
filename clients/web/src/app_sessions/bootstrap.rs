use leptos::{prelude::*, task::spawn_local};
use serde_json::json;

use super::AppSessionActions;
use crate::{
    api::{get_json, load_selected_session_dir, post_json, requested_session_dir},
    events::reconnect_event_stream,
    types::{BootstrapInfo, ResumeSessionRequest, StdioOutput, TransportStatus},
};

impl AppSessionActions {
    pub(crate) fn initialize(self) {
        let startup_generation = self.transcript.transcript_generation.get_untracked();
        spawn_local(async move {
            let bootstrap = match get_json::<BootstrapInfo>("/bootstrap").await {
                Ok(bootstrap) => bootstrap,
                Err(error) => {
                    if self.transcript.transcript_generation.get_untracked() != startup_generation {
                        return;
                    }
                    self.set_sidebar_sessions_status
                        .set(format!("не удалось подключиться: {error}"));
                    self.runtime_settings
                        .set_transport_status
                        .set(TransportStatus::Error(error));
                    return;
                }
            };
            if self.transcript.transcript_generation.get_untracked() != startup_generation {
                return;
            }
            self.runtime_settings
                .set_workspace_label
                .set(bootstrap.cwd.to_string_lossy().into_owned());
            let selected = requested_session_dir()
                .or_else(|| load_selected_session_dir().ok().flatten())
                .or(bootstrap
                    .session_dir
                    .map(|p| p.to_string_lossy().into_owned()));
            let result = match selected {
                Some(session_dir) => resume_session(session_dir).await,
                None => create_session(None).await,
            };
            if self.transcript.transcript_generation.get_untracked() != startup_generation {
                return;
            }
            match result {
                Ok(session_dir) => {
                    self.activate_session(session_dir.clone());
                    self.runtime_settings
                        .load(session_dir.clone(), startup_generation);
                    reconnect_event_stream(self.event_source, self.event_stream);
                    self.load_sidebar_sessions();
                }
                Err(error) => {
                    self.set_sidebar_sessions_status
                        .set(format!("не удалось открыть сессию: {error}"));
                    self.runtime_settings
                        .set_transport_status
                        .set(TransportStatus::Error(error));
                }
            }
        });
    }
}

async fn resume_session(session_dir: String) -> Result<String, String> {
    match post_json(
        "/resume",
        &ResumeSessionRequest {
            id: Some("startup-resume".to_owned()),
            session_dir: session_dir.clone().into(),
        },
    )
    .await
    {
        Ok(StdioOutput::Response { ok: true, .. }) => Ok(session_dir),
        Ok(StdioOutput::Response { error, .. }) => {
            Err(error.unwrap_or_else(|| "не удалось открыть выбранную сессию".to_owned()))
        }
        Ok(StdioOutput::Event { .. }) => Err("неожиданное событие resume".to_owned()),
        Err(error) => Err(error),
    }
}

pub(super) async fn create_session(source_session_dir: Option<String>) -> Result<String, String> {
    let mut request = json!({ "id": "new-session" });
    if let Some(source_session_dir) = source_session_dir {
        request["source_session_dir"] = json!(source_session_dir);
    }
    match post_json("/new-session", &request).await {
        Ok(StdioOutput::Response {
            ok: true,
            output: Some(output),
            ..
        }) => output
            .get("session_dir")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| "new-session response has no session_dir".to_owned()),
        Ok(StdioOutput::Response { error, .. }) => {
            Err(error.unwrap_or_else(|| "не удалось создать сессию".to_owned()))
        }
        Ok(StdioOutput::Event { .. }) => Err("неожиданное событие new-session".to_owned()),
        Err(error) => Err(error),
    }
}
