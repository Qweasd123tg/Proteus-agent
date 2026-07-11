use leptos::{html, prelude::*, task::spawn_local};
use serde_json::Value;
use wasm_bindgen::{JsCast, closure::Closure, prelude::wasm_bindgen};
use web_sys::{HtmlElement, HtmlTextAreaElement, window};

use crate::api::{encode_query_component, get_json};
use crate::messages::{adopt_streaming_tail, prepend_history_messages, report_error};
use crate::tool_names::{FOLLOWUP_TASK_TOOL, SPAWN_AGENT_TOOL, TASK_TOOL};
use crate::types::*;
use crate::ui_utils::{compact_text, compact_title, format_json};

pub(crate) const CHAT_REATTACH_THRESHOLD_PX: i32 = 4;
const CONTEXT_USAGE_STORAGE_PREFIX: &str = "proteus.contextUsage:";

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
    set_model_options: WriteSignal<Vec<String>>,
    set_reasoning_enabled: WriteSignal<bool>,
    set_effort: WriteSignal<ReasoningEffort>,
    set_effort_options: WriteSignal<Vec<String>>,
    set_workspace_label: WriteSignal<String>,
    set_active_session_dir: WriteSignal<Option<String>>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    set_agent_status: WriteSignal<String>,
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
                        set_active_turn_id,
                        set_agent_status,
                    );
                }
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
                let reasoning_enabled = config
                    .pointer("/reasoning/enabled")
                    .and_then(Value::as_bool);
                if let Some(enabled) = reasoning_enabled {
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
                    // «none» и «auto» — служебные значения, кнопка none в меню
                    // своя; из опций сервера их не дублируем.
                    .filter(|value| {
                        !value.eq_ignore_ascii_case("none") && !value.eq_ignore_ascii_case("auto")
                    })
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                if let Some(effort) = current_effort {
                    if !effort_options.iter().any(|item| item == effort) {
                        effort_options.push(effort.to_owned());
                    }
                    set_effort.set(ReasoningEffort::from_value(effort));
                } else if reasoning_enabled == Some(false) {
                    // Рассуждения выключены — в меню effort подсвечиваем none.
                    set_effort.set(ReasoningEffort::None);
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn load_transcript(
    messages: ReadSignal<Vec<Message>>,
    set_messages: WriteSignal<Vec<Message>>,
    transcript_generation: ReadSignal<u64>,
    expected_generation: u64,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    let expected_next_message_id = next_message_id.get_untracked();
    spawn_local(async move {
        match get_json::<Vec<TranscriptMessage>>("/history").await {
            Ok(items) => {
                if transcript_generation.get_untracked() != expected_generation {
                    return;
                }
                let transcript = transcript_messages(items);
                if transcript.is_empty() {
                    return;
                }
                if messages.get_untracked().is_empty()
                    && next_message_id.get_untracked() == expected_next_message_id
                {
                    set_next_message_id.set(next_message_id_after(&transcript));
                    adopt_streaming_tail(
                        &transcript,
                        set_active_stream_message_id,
                        set_streamed_this_turn,
                    );
                    set_messages.set(transcript);
                } else {
                    // Агент пишет: SSE доставил живые сообщения раньше, чем
                    // пришёл /history. Историю не выбрасываем (иначе лента
                    // теряет все прошлые ходы до конца текущего), а
                    // подкладываем перед живым хвостом.
                    prepend_history_messages(
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        set_active_stream_message_id,
                        set_streamed_this_turn,
                        transcript,
                    );
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

/// Транскрипт с сервера → сообщения ленты. Два subagent-шва повторяют
/// live-путь, чтобы вид после reload совпадал с живым:
/// - снапшот прогресса шлёт карточку субагента отдельным сообщением сразу
///   после его вызова `task`/`spawn_agent`/`followup_task` — сливаем в одну
///   карточку (как SubagentStarted прикрепляется к бегущей facade-карточке);
/// - committed history карточек субагента не хранит — восстанавливаем вид из
///   завершённого вызова `task` (args + metadata результата).
fn transcript_messages(items: Vec<TranscriptMessage>) -> Vec<Message> {
    let mut messages: Vec<Message> = Vec::with_capacity(items.len());
    let mut synthetic: Vec<Option<SubagentActivity>> = Vec::with_capacity(items.len());
    for item in items {
        let candidate = item
            .tool
            .as_ref()
            .and_then(subagent_from_task_transcript_tool);
        let tool = item.tool.map(transcript_tool_activity);
        let subagent = item.subagent.map(transcript_subagent_activity);
        if let Some(activity) = subagent {
            if item.text.trim().is_empty() && tool.is_none() {
                let matching_parent = messages.iter_mut().rev().find(|message| {
                    message.subagent.is_none()
                        && message.tool.as_ref().is_some_and(|tool| {
                            if tool.name == TASK_TOOL {
                                return true;
                            }
                            activity.description.as_deref().is_some_and(|task_name| {
                                collaboration_parent_matches(tool, task_name)
                            })
                        })
                });
                if let Some(parent) = matching_parent {
                    parent.subagent = Some(activity);
                    continue;
                }
            }
            messages.push(Message {
                id: 0,
                version: 0,
                role: message_role_from_wire(&item.role),
                text: item.text,
                tool,
                subagent: Some(activity),
                streaming: item.streaming,
            });
            synthetic.push(None);
            continue;
        }
        messages.push(Message {
            id: 0,
            version: 0,
            role: message_role_from_wire(&item.role),
            text: item.text,
            tool,
            subagent: None,
            streaming: item.streaming,
        });
        synthetic.push(candidate);
    }
    // Настоящая карточка (из событий) авторитетнее реконструкции: synthetic
    // не ставим, если тот же child_thread_id уже есть в ленте.
    let known: Vec<String> = messages
        .iter()
        .filter_map(|message| {
            message
                .subagent
                .as_ref()
                .map(|subagent| subagent.child_thread_id.clone())
        })
        .collect();
    for (message, candidate) in messages.iter_mut().zip(synthetic) {
        if message.subagent.is_none()
            && let Some(candidate) = candidate
            && !known.contains(&candidate.child_thread_id)
        {
            message.subagent = Some(candidate);
        }
    }
    for (index, message) in messages.iter_mut().enumerate() {
        message.id = index as u64 + 1;
    }
    messages
}

fn collaboration_parent_matches(tool: &ToolActivity, task_name: &str) -> bool {
    match tool.name.as_str() {
        SPAWN_AGENT_TOOL => {
            tool.args.get("task_name").and_then(Value::as_str) == Some(task_name)
        }
        FOLLOWUP_TASK_TOOL => tool
            .args
            .get("target")
            .and_then(Value::as_str)
            .is_some_and(|target| target == task_name || target == format!("/root/{task_name}")),
        _ => false,
    }
}

/// Реконструкция карточки субагента из завершённого вызова `task`:
/// роль/описание — из args, статус и итерации — из metadata результата
/// (форма `SubagentResult`, см. task_tool в coding-workflow). Без
/// child_thread_id в metadata (ошибка, бегущий вызов) карточки нет.
fn subagent_from_task_transcript_tool(tool: &TranscriptTool) -> Option<SubagentActivity> {
    if tool.name != TASK_TOOL {
        return None;
    }
    let child_thread_id = tool
        .metadata
        .get("child_thread_id")
        .and_then(Value::as_str)?;
    let status = tool
        .metadata
        .get("status")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            if tool.status == "done" {
                "completed".to_owned()
            } else {
                "errored".to_owned()
            }
        });
    Some(SubagentActivity {
        child_thread_id: child_thread_id.to_owned(),
        role: tool
            .args
            .get("agent_type")
            .and_then(Value::as_str)
            .unwrap_or("subagent")
            .to_owned(),
        description: tool
            .args
            .get("description")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        status: SubagentActivityStatus::Finished(status),
        iterations: tool
            .metadata
            .get("iterations")
            .and_then(Value::as_u64)
            .map(|iterations| iterations.min(u64::from(u32::MAX)) as u32),
        started_at_ms: 0,
        finished_at_ms: None,
        tools: Vec::new(),
    })
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn replace_transcript(
    set_messages: WriteSignal<Vec<Message>>,
    transcript_generation: ReadSignal<u64>,
    expected_generation: u64,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    replace_transcript_for_session(
        None,
        set_messages,
        transcript_generation,
        expected_generation,
        next_message_id,
        set_next_message_id,
        set_active_stream_message_id,
        set_streamed_this_turn,
        set_transport_status,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn replace_transcript_for_session(
    session_dir: Option<String>,
    set_messages: WriteSignal<Vec<Message>>,
    transcript_generation: ReadSignal<u64>,
    expected_generation: u64,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
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
                adopt_streaming_tail(
                    &transcript,
                    set_active_stream_message_id,
                    set_streamed_this_turn,
                );
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
        crate::ui_utils::now_ms()
    } else {
        0
    };
    ToolActivity {
        call_id: tool.call_id,
        name: tool.name,
        args: tool.args.clone(),
        args_preview: format_json(&tool.args),
        started_at_ms,
        // Истории момент старта неизвестен — duration не восстанавливаем.
        finished_at_ms: None,
        status,
        result_preview: tool.result,
    }
}

fn transcript_subagent_activity(subagent: TranscriptSubagent) -> SubagentActivity {
    let status = if subagent.status == "running" {
        SubagentActivityStatus::Running
    } else {
        SubagentActivityStatus::Finished(subagent.status)
    };
    let started_at_ms = if matches!(status, SubagentActivityStatus::Running) {
        crate::ui_utils::now_ms()
    } else {
        0
    };
    SubagentActivity {
        child_thread_id: subagent.child_thread_id,
        role: subagent.role,
        description: subagent.description,
        status,
        iterations: subagent.iterations,
        started_at_ms,
        finished_at_ms: None,
        tools: subagent
            .tools
            .into_iter()
            .map(transcript_tool_activity)
            .map(|mut tool| {
                // Тот же потолок, что у live-пути: восстановленная карточка
                // не должна снова раздуться до мегабайтов.
                if let Some(result_preview) = tool.result_preview.as_deref() {
                    tool.result_preview = Some(compact_text(
                        result_preview,
                        crate::messages::NESTED_TOOL_PREVIEW_CHAR_LIMIT,
                    ));
                }
                tool
            })
            .collect(),
    }
}

fn tool_status_from_wire(status: &str) -> ToolActivityStatus {
    match status {
        "waiting_approval" => ToolActivityStatus::WaitingApproval,
        "approved" => ToolActivityStatus::Approved,
        "denied" => ToolActivityStatus::Denied,
        "done" => ToolActivityStatus::Done,
        "failed" => ToolActivityStatus::Failed,
        "interrupted" => ToolActivityStatus::Interrupted,
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

/// id активного пользовательского сообщения для подсветки в миникарте: последнее,
/// чья верхняя граница уже выше верха ленты (т.е. чью секцию сейчас читаешь).
pub(crate) fn active_user_message_id(items: &[(u64, String)], container_top: f64) -> Option<u64> {
    let document = window().and_then(|window| window.document())?;
    let mut active = items.first().map(|(id, _)| *id);
    for (id, _) in items {
        if let Some(element) = document.get_element_by_id(&format!("msg-{id}")) {
            if element.get_bounding_client_rect().top() <= container_top + 40.0 {
                active = Some(*id);
            } else {
                break;
            }
        }
    }
    active
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

/// Черновики композера: по ключу на сессию, переживают переключение и F5.
const DRAFT_STORAGE_PREFIX: &str = "proteus.draft:";

pub(crate) fn load_session_draft(session_dir: &str) -> Option<String> {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| {
            storage
                .get_item(&format!("{DRAFT_STORAGE_PREFIX}{session_dir}"))
                .ok()
                .flatten()
        })
        .filter(|value| !value.trim().is_empty())
}

pub(crate) fn save_session_draft(session_dir: &str, draft: &str) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let key = format!("{DRAFT_STORAGE_PREFIX}{session_dir}");
        if draft.trim().is_empty() {
            let _ = storage.remove_item(&key);
        } else {
            let _ = storage.set_item(&key, draft);
        }
    }
}

pub(crate) fn remove_session_draft(session_dir: &str) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.remove_item(&format!("{DRAFT_STORAGE_PREFIX}{session_dir}"));
    }
}

/// Снимок контекста ключуется по сессии. Глобальный ключ здесь опасен:
/// после смены сессии — или модуля workflow/компактора, который перестал
/// слать TokenUsageUpdated, — бублик показывал бы хвост чужой сессии.
pub(crate) fn load_context_usage(session_dir: Option<&str>) -> Option<ContextUsage> {
    let session_dir = session_dir?;
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| {
            storage
                .get_item(&format!("{CONTEXT_USAGE_STORAGE_PREFIX}{session_dir}"))
                .ok()
                .flatten()
        })
        .and_then(|value| serde_json::from_str(&value).ok())
}

pub(crate) fn save_context_usage(session_dir: Option<&str>, usage: ContextUsage) {
    let Some(session_dir) = session_dir else {
        return;
    };
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten())
        && let Ok(value) = serde_json::to_string(&usage)
    {
        let _ = storage.set_item(
            &format!("{CONTEXT_USAGE_STORAGE_PREFIX}{session_dir}"),
            &value,
        );
    }
}

pub(crate) fn remove_context_usage(session_dir: &str) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.remove_item(&format!("{CONTEXT_USAGE_STORAGE_PREFIX}{session_dir}"));
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
    // Untracked: это управляющий флаг «кадр уже запланирован», а не
    // зависимость. Tracked-чтение подписывало вызывающий эффект автоскролла
    // на сам флаг: set(true) → rerun, set(false) в rAF → rerun → новый кадр →
    // set(true) → ... — вечный 60fps-цикл с принудительным reflow всей ленты
    // (scroll_height) на каждом кадре, который и вешал вкладку на длинных
    // транскриптах во время стрима.
    if scroll_frame_pending.get_untracked() {
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
    // rAF-колбэк — не реактивный контекст: tracked-чтения здесь бессмысленны
    // и в dev-сборке заваливают консоль предупреждениями reactive_graph.
    if let Some(results) = results_ref.get_untracked()
        && stick_to_bottom.get_untracked()
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
    fn transcript_messages_merge_progress_subagent_into_preceding_task_card() {
        // Снапшот прогресса: карточка бегущего task + отдельное
        // subagent-сообщение сразу за ней — как шлёт turn_progress.
        let messages = transcript_messages(vec![
            TranscriptMessage {
                role: "system".to_owned(),
                text: String::new(),
                tool: Some(TranscriptTool {
                    call_id: "call-task".to_owned(),
                    name: "task".to_owned(),
                    args: serde_json::json!({"agent_type": "explore", "prompt": "look around"}),
                    status: "running".to_owned(),
                    result: None,
                    metadata: Value::Null,
                }),
                subagent: None,
                streaming: false,
            },
            TranscriptMessage {
                role: "system".to_owned(),
                text: String::new(),
                tool: None,
                subagent: Some(TranscriptSubagent {
                    child_thread_id: "child-thread".to_owned(),
                    role: "explore".to_owned(),
                    description: None,
                    status: "running".to_owned(),
                    iterations: None,
                    tools: Vec::new(),
                }),
                streaming: false,
            },
        ]);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, 1);
        let tool = messages[0].tool.as_ref().expect("task tool");
        assert_eq!(tool.call_id, "call-task");
        let subagent = messages[0].subagent.as_ref().expect("merged subagent");
        assert_eq!(subagent.child_thread_id, "child-thread");
        assert!(subagent.is_running());
    }

    #[test]
    fn transcript_messages_merge_background_subagent_into_matching_spawn_card() {
        let messages = transcript_messages(vec![
            TranscriptMessage {
                role: "system".to_owned(),
                text: String::new(),
                tool: Some(TranscriptTool {
                    call_id: "call-spawn".to_owned(),
                    name: SPAWN_AGENT_TOOL.to_owned(),
                    args: serde_json::json!({
                        "task_name": "scan",
                        "message": "look around",
                        "agent_type": "explore"
                    }),
                    status: "done".to_owned(),
                    result: Some("started".to_owned()),
                    metadata: Value::Null,
                }),
                subagent: None,
                streaming: false,
            },
            TranscriptMessage {
                role: "assistant".to_owned(),
                text: "Продолжаю основной ход".to_owned(),
                tool: None,
                subagent: None,
                streaming: true,
            },
            TranscriptMessage {
                role: "system".to_owned(),
                text: String::new(),
                tool: None,
                subagent: Some(TranscriptSubagent {
                    child_thread_id: "child-thread".to_owned(),
                    role: "explore".to_owned(),
                    description: Some("scan".to_owned()),
                    status: "running".to_owned(),
                    iterations: None,
                    tools: Vec::new(),
                }),
                streaming: false,
            },
        ]);

        assert_eq!(messages.len(), 2);
        assert!(messages[0].subagent.is_some());
        assert!(messages[1].streaming);
        assert_eq!(messages[1].text, "Продолжаю основной ход");
    }

    #[test]
    fn transcript_messages_merge_background_subagent_into_matching_followup_card() {
        let messages = transcript_messages(vec![
            TranscriptMessage {
                role: "system".to_owned(),
                text: String::new(),
                tool: Some(TranscriptTool {
                    call_id: "call-followup".to_owned(),
                    name: FOLLOWUP_TASK_TOOL.to_owned(),
                    args: serde_json::json!({
                        "target": "/root/scan",
                        "message": "continue"
                    }),
                    status: "done".to_owned(),
                    result: Some("resumed".to_owned()),
                    metadata: Value::Null,
                }),
                subagent: None,
                streaming: false,
            },
            TranscriptMessage {
                role: "system".to_owned(),
                text: String::new(),
                tool: None,
                subagent: Some(TranscriptSubagent {
                    child_thread_id: "child-thread".to_owned(),
                    role: "explore".to_owned(),
                    description: Some("scan".to_owned()),
                    status: "running".to_owned(),
                    iterations: None,
                    tools: Vec::new(),
                }),
                streaming: false,
            },
        ]);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].tool.as_ref().map(|tool| tool.name.as_str()),
            Some(FOLLOWUP_TASK_TOOL)
        );
        assert!(messages[0].subagent.is_some());
    }

    #[test]
    fn transcript_messages_reconstruct_subagent_from_committed_task_result() {
        // Committed history: карточек субагента нет, но у результата task
        // есть metadata SubagentResult — карточка восстанавливается из неё.
        let messages = transcript_messages(vec![TranscriptMessage {
            role: "system".to_owned(),
            text: String::new(),
            tool: Some(TranscriptTool {
                call_id: "call-task".to_owned(),
                name: "task".to_owned(),
                args: serde_json::json!({
                    "agent_type": "explore",
                    "description": "map the crate",
                    "prompt": "look around"
                }),
                status: "done".to_owned(),
                result: Some("summary text".to_owned()),
                metadata: serde_json::json!({
                    "status": "completed",
                    "iterations": 3,
                    "child_thread_id": "child-thread"
                }),
            }),
            subagent: None,
            streaming: false,
        }]);

        assert_eq!(messages.len(), 1);
        let subagent = messages[0].subagent.as_ref().expect("synthetic subagent");
        assert_eq!(subagent.role, "explore");
        assert_eq!(subagent.description.as_deref(), Some("map the crate"));
        assert_eq!(
            subagent.status,
            SubagentActivityStatus::Finished("completed".to_owned())
        );
        assert_eq!(subagent.iterations, Some(3));
        // Итог виден через result_preview слитой tool-карточки.
        assert_eq!(
            messages[0]
                .tool
                .as_ref()
                .and_then(|tool| tool.result_preview.as_deref()),
            Some("summary text")
        );
    }

    #[test]
    fn transcript_messages_skip_reconstruction_for_failed_task_without_metadata() {
        let messages = transcript_messages(vec![TranscriptMessage {
            role: "system".to_owned(),
            text: String::new(),
            tool: Some(TranscriptTool {
                call_id: "call-task".to_owned(),
                name: "task".to_owned(),
                args: serde_json::json!({"agent_type": "explore", "prompt": "look"}),
                status: "failed".to_owned(),
                result: Some("boom".to_owned()),
                metadata: serde_json::json!({"tool": "task"}),
            }),
            subagent: None,
            streaming: false,
        }]);

        // Без child_thread_id карточку не к чему привязать — остаётся
        // обычная tool-карточка с ошибкой.
        assert_eq!(messages.len(), 1);
        assert!(messages[0].subagent.is_none());
        assert!(messages[0].tool.is_some());
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
                metadata: Value::Null,
            }),
            subagent: None,
            streaming: false,
        }]);

        assert_eq!(messages.len(), 1);
        let tool = messages[0].tool.as_ref().expect("tool activity");
        assert_eq!(tool.call_id, "call-1");
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.args_preview, "{\n  \"path\": \"src/lib.rs\"\n}");
        assert_eq!(tool.status, ToolActivityStatus::Done);
        assert_eq!(tool.result_preview.as_deref(), Some("line 1\nline 2"));
    }

    #[test]
    fn transcript_messages_restore_subagent_activity_cards() {
        let messages = transcript_messages(vec![TranscriptMessage {
            role: "system".to_owned(),
            text: String::new(),
            tool: None,
            subagent: Some(TranscriptSubagent {
                child_thread_id: "child-thread".to_owned(),
                role: "reviewer".to_owned(),
                description: Some("check patch".to_owned()),
                status: "completed".to_owned(),
                iterations: Some(2),
                tools: vec![TranscriptTool {
                    call_id: "call-child".to_owned(),
                    name: "read_file".to_owned(),
                    args: serde_json::json!({"path": "src/lib.rs"}),
                    status: "done".to_owned(),
                    result: Some("contents".to_owned()),
                    metadata: Value::Null,
                }],
            }),
            streaming: false,
        }]);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].tool.is_none());
        let subagent = messages[0].subagent.as_ref().expect("subagent activity");
        assert_eq!(subagent.child_thread_id, "child-thread");
        assert_eq!(subagent.role, "reviewer");
        assert_eq!(subagent.description.as_deref(), Some("check patch"));
        assert_eq!(
            subagent.status,
            SubagentActivityStatus::Finished("completed".to_owned())
        );
        assert_eq!(subagent.iterations, Some(2));
        assert_eq!(subagent.tools.len(), 1);
        assert_eq!(subagent.tools[0].call_id, "call-child");
        assert_eq!(subagent.tools[0].status, ToolActivityStatus::Done);
        assert_eq!(
            subagent.tools[0].result_preview.as_deref(),
            Some("contents")
        );
    }
}
