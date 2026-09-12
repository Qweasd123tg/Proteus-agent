use crate::{
    api::{get_json, session_path},
    messages::report_error,
    types::*,
};
use leptos::{prelude::*, task::spawn_local};
use proteus_contracts::app_protocol::config::ConfigSummary;

#[allow(clippy::too_many_arguments)]
pub(crate) fn load_runtime_settings(
    session_dir: String,
    active_session_dir: ReadSignal<Option<String>>,
    transcript_generation: ReadSignal<u64>,
    expected_generation: u64,
    set_mode: WriteSignal<PermissionMode>,
    set_model_name: WriteSignal<String>,
    set_model_options: WriteSignal<Vec<ModelOption>>,
    set_reasoning_enabled: WriteSignal<bool>,
    set_effort: WriteSignal<ReasoningEffort>,
    set_effort_options: WriteSignal<Vec<String>>,
    set_workspace_label: WriteSignal<String>,
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    spawn_local(async move {
        let result = get_json::<ConfigSummary>(&session_path("/config", &session_dir)).await;
        if transcript_generation.get_untracked() != expected_generation
            || active_session_dir.get_untracked().as_deref() != Some(session_dir.as_str())
        {
            return;
        }
        match result {
            Ok(config) => {
                set_workspace_label.set(config.cwd.clone());
                set_mode.set(PermissionMode::from_value(&config.permission_mode));
                crate::model_settings::ModelSettings {
                    model: set_model_name,
                    models: set_model_options,
                    enabled: set_reasoning_enabled,
                    effort: set_effort,
                    efforts: set_effort_options,
                    status: set_transport_status,
                }
                .apply(&config);
            }
            Err(error) => report_error(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_transport_status,
                "Config load failed",
                error,
            ),
        }
    });
}
