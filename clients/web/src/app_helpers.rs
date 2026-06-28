use leptos::{html, prelude::*, task::spawn_local};
use serde_json::Value;
use wasm_bindgen::{JsCast, closure::Closure, prelude::wasm_bindgen};
use web_sys::{HtmlElement, HtmlTextAreaElement, window};

use crate::api::{encode_query_component, get_json};
use crate::messages::report_error;
use crate::types::*;
use crate::ui_utils::{compact_text, compact_title, format_json};

pub(crate) const CHAT_REATTACH_THRESHOLD_PX: i32 = 4;
const CONTEXT_USAGE_STORAGE_KEY: &str = "proteus.contextUsage";

#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = requestAnimationFrame)]
    fn request_animation_frame(callback: &js_sys::Function) -> i32;
}

pub(crate) fn insert_textarea_newline(
    textarea: HtmlTextAreaElement,
    set_draft: WriteSignal<String>,
) {
    let value = textarea.value();
    let start = textarea
        .selection_start()
        .ok()
        .flatten()
        .unwrap_or(value.encode_utf16().count() as u32);
    let end = textarea.selection_end().ok().flatten().unwrap_or(start);
    let start_index = utf16_offset_to_byte_index(&value, start);
    let end_index = utf16_offset_to_byte_index(&value, end);
    let mut next = String::with_capacity(value.len() + 1);
    next.push_str(&value[..start_index]);
    next.push('\n');
    next.push_str(&value[end_index..]);
    let next_cursor = start + 1;

    textarea.set_value(&next);
    let _ = textarea.set_selection_start(Some(next_cursor));
    let _ = textarea.set_selection_end(Some(next_cursor));
    set_draft.set(next);
}

fn utf16_offset_to_byte_index(text: &str, offset: u32) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= offset {
            return index;
        }
        units += ch.len_utf16() as u32;
    }
    text.len()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn load_runtime_settings(
    set_mode: WriteSignal<PermissionMode>,
    set_model_name: WriteSignal<String>,
    set_model_options: WriteSignal<Vec<String>>,
    set_reasoning_enabled: WriteSignal<bool>,
    set_effort: WriteSignal<ReasoningEffort>,
    set_effort_options: WriteSignal<Vec<String>>,
    set_workspace_label: WriteSignal<String>,
    set_active_session_dir: WriteSignal<Option<String>>,
    set_messages: WriteSignal<Vec<Message>>,
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
                if let Some(mode) = config.get("permission_mode").and_then(Value::as_str) {
                    set_mode.set(PermissionMode::from_value(mode));
                }
                if let Some(model) = config.pointer("/model/name").and_then(Value::as_str) {
                    set_model_name.set(model.to_owned());
                }
                let mut options = config
                    .get("model_options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| {
                        item.get("name")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                    .collect::<Vec<_>>();
                if let Some(model) = config.pointer("/model/name").and_then(Value::as_str)
                    && !options.iter().any(|item| item == model)
                {
                    options.push(model.to_owned());
                }
                set_model_options.set(options);
                if let Some(enabled) = config
                    .pointer("/reasoning/enabled")
                    .and_then(Value::as_bool)
                {
                    set_reasoning_enabled.set(enabled);
                }
                let current_effort = config.pointer("/reasoning/effort").and_then(Value::as_str);
                let mut effort_options = config
                    .pointer("/reasoning/effort_options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                if let Some(effort) = current_effort {
                    if !effort_options.iter().any(|item| item == effort) {
                        effort_options.push(effort.to_owned());
                    }
                    set_effort.set(ReasoningEffort::from_value(effort));
                }
                set_effort_options.set(effort_options);
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

pub(crate) fn load_transcript(
    messages: ReadSignal<Vec<Message>>,
    set_messages: WriteSignal<Vec<Message>>,
    transcript_generation: ReadSignal<u64>,
    expected_generation: u64,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    let expected_next_message_id = next_message_id.get_untracked();
    spawn_local(async move {
        match get_json::<Vec<TranscriptMessage>>("/history").await {
            Ok(items) => {
                let transcript = transcript_messages(items);
                if !transcript.is_empty()
                    && transcript_generation.get_untracked() == expected_generation
                    && next_message_id.get_untracked() == expected_next_message_id
                    && messages.get_untracked().is_empty()
                {
                    set_next_message_id.set(next_message_id_after(&transcript));
                    set_messages.set(transcript);
                }
            }
            Err(error) => report_error(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_transport_status,
                "History load failed",
                error,
            ),
        }
    });
}

fn transcript_messages(items: Vec<TranscriptMessage>) -> Vec<Message> {
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let tool = item.tool.map(transcript_tool_activity);
            Message {
                id: index as u64 + 1,
                version: 0,
                role: message_role_from_wire(&item.role),
                text: item.text,
                tool,
                streaming: false,
            }
        })
        .collect()
}

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
    pub(crate) active_turn_id: Option<String>,
    pub(crate) agent_status: String,
}

pub(crate) fn active_session_activity_state(
    activity: Option<&SessionActivityInfo>,
) -> ActiveSessionActivityState {
    let is_sending = activity.is_some_and(session_activity_is_busy);
    let active_turn_id = activity
        .and_then(|activity| activity.running_turn_ids.first())
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
        active_turn_id,
        agent_status,
    }
}

pub(crate) fn apply_active_session_activity(
    activity: Option<&SessionActivityInfo>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    set_agent_status: WriteSignal<String>,
) {
    let state = active_session_activity_state(activity);
    set_is_sending.set(state.is_sending);
    set_active_turn_id.set(state.active_turn_id);
    set_agent_status.set(state.agent_status);
}

fn session_activity_is_busy(activity: &SessionActivityInfo) -> bool {
    activity.running_turns > 0
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
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        session.session_dir,
        session.message_count,
        session.updated_at_ms.unwrap_or_default(),
        session.preview.as_deref().unwrap_or_default(),
        session.resumable,
        activity
            .map(|activity| activity.status.as_str())
            .unwrap_or(""),
        activity.map(|activity| activity.running_turns).unwrap_or(0),
        activity
            .map(|activity| activity.running_turn_ids.join(","))
            .unwrap_or_default(),
        activity
            .map(|activity| activity.pending_approvals + activity.pending_user_inputs)
            .unwrap_or(0),
    )
}

pub(crate) fn replace_transcript(
    set_messages: WriteSignal<Vec<Message>>,
    transcript_generation: ReadSignal<u64>,
    expected_generation: u64,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    replace_transcript_for_session(
        None,
        set_messages,
        transcript_generation,
        expected_generation,
        next_message_id,
        set_next_message_id,
        set_transport_status,
    );
}

pub(crate) fn replace_transcript_for_session(
    session_dir: Option<String>,
    set_messages: WriteSignal<Vec<Message>>,
    transcript_generation: ReadSignal<u64>,
    expected_generation: u64,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    spawn_local(async move {
        match get_json::<Vec<TranscriptMessage>>(&history_path(session_dir.as_deref())).await {
            Ok(items) => {
                if transcript_generation.get_untracked() != expected_generation {
                    return;
                }
                let transcript = transcript_messages(items);
                set_next_message_id.set(next_message_id_after(&transcript));
                set_messages.set(transcript);
            }
            Err(error) => report_error(
                set_messages,
                next_message_id,
                set_next_message_id,
                set_transport_status,
                "History load failed",
                error,
            ),
        }
    });
}

fn history_path(session_dir: Option<&str>) -> String {
    match session_dir {
        Some(session_dir) => format!(
            "/history?session_dir={}",
            encode_query_component(session_dir)
        ),
        None => "/history".to_owned(),
    }
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

fn message_role_from_wire(role: &str) -> MessageRole {
    match role {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        _ => MessageRole::System,
    }
}

fn transcript_tool_activity(tool: TranscriptTool) -> ToolActivity {
    let status = tool_status_from_wire(&tool.status);
    let started_at_ms = if matches!(
        status,
        ToolActivityStatus::Running
            | ToolActivityStatus::WaitingApproval
            | ToolActivityStatus::Approved
    ) {
        js_sys::Date::now().max(0.0) as u64
    } else {
        0
    };
    ToolActivity {
        call_id: tool.call_id,
        name: tool.name,
        args: tool.args.clone(),
        args_preview: format_json(&tool.args),
        started_at_ms,
        status,
        result_preview: tool.result,
    }
}

fn tool_status_from_wire(status: &str) -> ToolActivityStatus {
    match status {
        "waiting_approval" => ToolActivityStatus::WaitingApproval,
        "approved" => ToolActivityStatus::Approved,
        "denied" => ToolActivityStatus::Denied,
        "done" => ToolActivityStatus::Done,
        "failed" => ToolActivityStatus::Failed,
        _ => ToolActivityStatus::Running,
    }
}

fn next_message_id_after(messages: &[Message]) -> u64 {
    messages.iter().map(|message| message.id).max().unwrap_or(0) + 1
}

pub(crate) fn current_path() -> String {
    window()
        .and_then(|window| window.location().pathname().ok())
        .unwrap_or_else(|| "/".to_owned())
}

pub(crate) fn load_i32_setting(key: &str, fallback: i32) -> i32 {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten())
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(fallback)
}

pub(crate) fn save_i32_setting(key: &str, value: i32) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, &value.to_string());
    }
}

pub(crate) fn load_bool_setting(key: &str, fallback: bool) -> bool {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten())
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(fallback)
}

pub(crate) fn save_bool_setting(key: &str, value: bool) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, if value { "true" } else { "false" });
    }
}

pub(crate) fn load_context_usage() -> Option<ContextUsage> {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(CONTEXT_USAGE_STORAGE_KEY).ok().flatten())
        .and_then(|value| serde_json::from_str(&value).ok())
}

pub(crate) fn save_context_usage(usage: ContextUsage) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten())
        && let Ok(value) = serde_json::to_string(&usage)
    {
        let _ = storage.set_item(CONTEXT_USAGE_STORAGE_KEY, &value);
    }
}

pub(crate) fn is_at_bottom(results: &HtmlElement) -> bool {
    let distance = results.scroll_height() - results.scroll_top() - results.client_height();
    distance <= CHAT_REATTACH_THRESHOLD_PX
}

pub(crate) fn latest_math_signature(messages: &[Message]) -> Option<(u64, u64)> {
    messages
        .iter()
        .rev()
        .find(|message| {
            !message.streaming && message.tool.is_none() && message_may_contain_math(&message.text)
        })
        .map(|message| (message.id, message.version))
}

fn message_may_contain_math(text: &str) -> bool {
    text.contains('$') || text.contains("\\(") || text.contains("\\[")
}

pub(crate) fn tool_activity_is_active(tool: &ToolActivity) -> bool {
    matches!(
        tool.status,
        ToolActivityStatus::Running
            | ToolActivityStatus::WaitingApproval
            | ToolActivityStatus::Approved
    )
}

pub(crate) fn schedule_results_scroll(
    results_ref: NodeRef<html::Section>,
    stick_to_bottom: ReadSignal<bool>,
    scroll_frame_pending: ReadSignal<bool>,
    set_scroll_frame_pending: WriteSignal<bool>,
    set_last_results_scroll_top: WriteSignal<i32>,
) {
    if scroll_frame_pending.get() {
        return;
    }
    set_scroll_frame_pending.set(true);

    let callback = Closure::<dyn FnMut()>::wrap(Box::new(move || {
        scroll_results_to_bottom(results_ref, stick_to_bottom, set_last_results_scroll_top);
        set_scroll_frame_pending.set(false);
    }));
    request_animation_frame(callback.as_ref().unchecked_ref());
    callback.forget();
}

fn scroll_results_to_bottom(
    results_ref: NodeRef<html::Section>,
    stick_to_bottom: ReadSignal<bool>,
    set_last_results_scroll_top: WriteSignal<i32>,
) {
    if let Some(results) = results_ref.get()
        && stick_to_bottom.get()
    {
        results.set_scroll_top(results.scroll_height());
        set_last_results_scroll_top.set(results.scroll_top());
    }
}

pub(crate) fn seed_messages() -> Vec<Message> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_summary(preview: Option<&str>, message_count: usize) -> SessionSummary {
        SessionSummary {
            session_dir: "/tmp/session".to_owned(),
            session_id: Some("1234567890".to_owned()),
            workspace_path: Some("/tmp/workspace".to_owned()),
            message_count,
            updated_at_ms: None,
            preview: preview.map(ToOwned::to_owned),
            resumable: true,
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
            running_turns: 1,
            running_turn_ids: vec!["turn-1".to_owned()],
            pending_approvals: 0,
            pending_user_inputs: 0,
        });

        assert_ne!(sidebar_session_render_key(&session), idle_key);
    }

    #[test]
    fn active_session_activity_restores_running_turn_state() {
        let activity = SessionActivityInfo {
            status: "running".to_owned(),
            running_turns: 1,
            running_turn_ids: vec!["turn-1".to_owned()],
            pending_approvals: 0,
            pending_user_inputs: 0,
        };

        assert_eq!(
            active_session_activity_state(Some(&activity)),
            ActiveSessionActivityState {
                is_sending: true,
                active_turn_id: Some("turn-1".to_owned()),
                agent_status: "работает".to_owned(),
            }
        );
    }

    #[test]
    fn active_session_activity_idle_clears_turn_state() {
        let activity = SessionActivityInfo {
            status: "idle".to_owned(),
            running_turns: 0,
            running_turn_ids: Vec::new(),
            pending_approvals: 0,
            pending_user_inputs: 0,
        };

        assert_eq!(
            active_session_activity_state(Some(&activity)),
            ActiveSessionActivityState {
                is_sending: false,
                active_turn_id: None,
                agent_status: "ожидает".to_owned(),
            }
        );
    }

    #[test]
    fn transcript_messages_restore_tool_activity_cards() {
        let messages = transcript_messages(vec![TranscriptMessage {
            role: "system".to_owned(),
            text: String::new(),
            tool: Some(TranscriptTool {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                args: serde_json::json!({"path": "src/lib.rs"}),
                status: "done".to_owned(),
                result: Some("line 1\nline 2".to_owned()),
            }),
        }]);

        assert_eq!(messages.len(), 1);
        let tool = messages[0].tool.as_ref().expect("tool activity");
        assert_eq!(tool.call_id, "call-1");
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.args_preview, "{\n  \"path\": \"src/lib.rs\"\n}");
        assert_eq!(tool.status, ToolActivityStatus::Done);
        assert_eq!(tool.result_preview.as_deref(), Some("line 1\nline 2"));
    }
}
