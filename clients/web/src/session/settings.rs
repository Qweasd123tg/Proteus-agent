use super::summaries::{apply_active_session_activity, session_activity_is_busy};
use crate::{api::get_json, messages::report_error, types::*};
use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;

#[allow(clippy::too_many_arguments)]
/// Разовая загрузка веб-настроек из секции [web] конфига (config_summary.web).
/// Отдельно от load_runtime_settings, чтобы не тащить параметр через её 4
/// вызова (они делят хвостовые аргументы с другими функциями).
pub(crate) fn load_web_settings(set_tool_cards_collapsed: WriteSignal<bool>) {
    spawn_local(async move {
        if let Ok(config) = get_json::<Value>("/config").await
            && let Some(collapsed) = config
                .pointer("/web/tool_cards_collapsed")
                .and_then(Value::as_bool)
        {
            set_tool_cards_collapsed.set(collapsed);
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn load_runtime_settings(
    set_mode: WriteSignal<PermissionMode>,
    set_model_name: WriteSignal<String>,
    set_model_options: WriteSignal<Vec<ModelOption>>,
    set_reasoning_enabled: WriteSignal<bool>,
    set_effort: WriteSignal<ReasoningEffort>,
    set_effort_options: WriteSignal<Vec<String>>,
    set_workspace_label: WriteSignal<String>,
    set_active_session_dir: WriteSignal<Option<String>>,
    set_is_sending: WriteSignal<bool>,
    set_active_run_id: WriteSignal<Option<String>>,
    set_agent_status: WriteSignal<String>,
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    spawn_local(async move {
        match get_json::<Value>("/config").await {
            Ok(config) => {
                if let Some(cwd) = config.get("cwd").and_then(Value::as_str) {
                    set_workspace_label.set(cwd.to_owned());
                }
                set_active_session_dir.set(
                    config
                        .get("session_dir")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                );
                // Сервер кладёт в /config activity текущей сессии. Если ход ещё
                // выполняется (страница открылась посреди хода), сразу помечаем
                // занятость: composer уводит новые сообщения в очередь, а не в
                // /send-async, и «Стоп» знает id бегущего хода. Idle нарочно не
                // применяем — не затирать оптимистичный is_sending уже начатой
                // отправки.
                if let Some(activity) = config
                    .get("activity")
                    .cloned()
                    .and_then(|value| serde_json::from_value::<SessionActivityInfo>(value).ok())
                    && session_activity_is_busy(&activity)
                {
                    apply_active_session_activity(
                        Some(&activity),
                        set_is_sending,
                        set_active_run_id,
                        set_agent_status,
                    );
                }
                if let Some(mode) = config.get("permission_mode").and_then(Value::as_str) {
                    set_mode.set(PermissionMode::from_value(mode));
                }
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
