use leptos::prelude::*;
use serde_json::Value;

use super::stream::{StreamFlushBindings, flush_stream_delta_buffer, queue_assistant_delta};
use crate::app_helpers::save_context_usage;
use crate::messages::{
    finish_active_streaming_assistant_message, finish_streaming_reasoning, push_message,
    push_tool_message, update_tool_status,
};
use crate::types::*;
use crate::ui_utils::{compact_text, format_json, short_id, short_path};

pub(crate) fn event_updates_visible_count(event: &AppServerEvent) -> bool {
    !matches!(
        event,
        AppServerEvent::Runtime { envelope }
            if runtime_event_is_stream_delta(envelope)
    )
}

pub(crate) fn update_session_labels(
    envelope: Value,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    set_active_session_dir: WriteSignal<Option<String>>,
) {
    let Some(started) = envelope.pointer("/event/SessionStarted") else {
        return;
    };
    if let Some(cwd) = started.get("cwd").and_then(Value::as_str) {
        set_workspace_label.set(cwd.to_owned());
    }
    if let Some(session_dir) = started.get("session_dir").and_then(Value::as_str) {
        set_active_session_dir.set(Some(session_dir.to_owned()));
        set_session_label.set(short_path(session_dir));
    } else if let Some(session_id) = started.get("session_id").and_then(Value::as_str) {
        set_session_label.set(short_id(session_id).to_owned());
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_runtime_status_and_tools(
    envelope: &Value,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    stream_bindings: StreamFlushBindings,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_context_usage: WriteSignal<Option<ContextUsage>>,
) {
    let Some(event) = envelope.get("event") else {
        return;
    };

    if let Some(usage_event) = event.get("TokenUsageUpdated") {
        if let Some(usage) = usage_event.get("usage").and_then(parse_context_usage) {
            save_context_usage(usage);
            set_context_usage.set(Some(usage));
        }
        return;
    }

    if event.get("TurnStarted").is_some() {
        flush_stream_delta_buffer(stream_bindings);
        stream_bindings.set_streamed_this_turn.set(false);
        stream_bindings.set_active_stream_message_id.set(None);
        finish_streaming_reasoning(set_messages);
        set_agent_status.set("начинает".to_owned());
    } else if event.get("TaskReceived").is_some() {
        set_agent_status.set("готовит задачу".to_owned());
    } else if event.get("HistoryCompactionStarted").is_some() {
        set_agent_status.set("сжимает историю".to_owned());
    } else if let Some(compaction_event) = event.get("HistoryCompactionCompleted") {
        let Some(report) = compaction_event.get("report") else {
            return;
        };
        let changed = report
            .get("changed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        set_agent_status.set(if changed {
            "история сжата".to_owned()
        } else {
            "история без сжатия".to_owned()
        });
        if changed {
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                compaction_report_text(report),
            );
        }
    } else if let Some(failed_event) = event.get("HistoryCompactionFailed") {
        set_agent_status.set("сжатие не удалось".to_owned());
        let message = failed_event
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown compaction error");
        push_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            MessageRole::System,
            format!("Сжатие истории не удалось: {}", compact_text(message, 500)),
        );
    } else if event.get("ContextBuilt").is_some() {
        set_agent_status.set("собирает контекст".to_owned());
    } else if event.get("ModelRequestPrepared").is_some() {
        set_agent_status.set("думает".to_owned());
    } else if let Some(delta_event) = event.get("AssistantTextDelta") {
        if !stream_bindings.streamed_this_turn.get_untracked() {
            set_agent_status.set("пишет".to_owned());
        }
        if let Some(text) = delta_event.get("text").and_then(Value::as_str) {
            queue_assistant_delta(stream_bindings, text);
        }
    } else if event.get("AssistantReasoningDelta").is_some() {
        // Reasoning streams can be very chatty. The working indicator already
        // says "думает"; storing every chunk in the transcript makes Firefox
        // clone and re-render a growing string while the user only needs the
        // final answer.
    } else if let Some(tool_event) = event.get("ToolCallRequested") {
        flush_stream_delta_buffer(stream_bindings);
        set_agent_status.set("запускает tool".to_owned());
        finish_active_streaming_assistant_message(
            set_messages,
            stream_bindings.active_stream_message_id,
            stream_bindings.set_active_stream_message_id,
        );
        finish_streaming_reasoning(set_messages);
        if let Some(call) = tool_event.get("call") {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_owned();
            let name = call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_owned();
            let args = call.get("args").cloned().unwrap_or(Value::Null);
            let args_preview = format_json(&args);
            let tool = ToolActivity {
                call_id: call_id.clone(),
                name,
                args,
                args_preview,
                started_at_ms: js_sys::Date::now().max(0.0) as u64,
                status: ToolActivityStatus::Running,
                result_preview: None,
            };
            push_tool_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                tool.clone(),
            );
            set_tool_activities.update(|items| {
                if !items.iter().any(|item| item.call_id == call_id) {
                    items.push(tool);
                    trim_tool_activities(items);
                }
            });
        }
    } else if let Some(approval_event) = event.get("ApprovalRequested") {
        set_agent_status.set("ждёт доступ".to_owned());
        if let Some(call_id) = approval_event.get("call_id").and_then(Value::as_str) {
            update_tool_status(
                set_tool_activities,
                set_messages,
                call_id,
                ToolActivityStatus::WaitingApproval,
                None,
            );
        }
    } else if let Some(approval_event) = event.get("ApprovalResolved") {
        let approved = approval_event
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        set_agent_status.set(if approved {
            "доступ разрешён".to_owned()
        } else {
            "доступ отклонён".to_owned()
        });
        if let Some(call_id) = approval_event.get("call_id").and_then(Value::as_str) {
            update_tool_status(
                set_tool_activities,
                set_messages,
                call_id,
                if approved {
                    ToolActivityStatus::Approved
                } else {
                    ToolActivityStatus::Denied
                },
                None,
            );
        }
    } else if let Some(tool_event) = event.get("ToolFinished") {
        set_agent_status.set("tool завершён".to_owned());
        if let Some(result) = tool_event.get("result") {
            let Some(call_id) = result.get("call_id").and_then(Value::as_str) else {
                return;
            };
            let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let preview = tool_result_text(result);
            update_tool_status(
                set_tool_activities,
                set_messages,
                call_id,
                if ok {
                    ToolActivityStatus::Done
                } else {
                    ToolActivityStatus::Failed
                },
                Some(preview),
            );
        }
    } else if event.get("TurnFinished").is_some() {
        flush_stream_delta_buffer(stream_bindings);
        finish_streaming_reasoning(set_messages);
        set_agent_status.set("ожидает".to_owned());
    } else if event.get("Error").is_some() {
        set_agent_status.set("ошибка".to_owned());
    }
}

fn runtime_event_is_stream_delta(envelope: &Value) -> bool {
    let Some(event) = envelope.get("event") else {
        return false;
    };
    event.get("AssistantTextDelta").is_some() || event.get("AssistantReasoningDelta").is_some()
}

/// Снимок заполнения контекста: за «использовано» берём реальный замер
/// провайдера, а при его отсутствии (фаза оценки) — локальную прикидку.
/// Без известного потолка окна бублик показывать нечем — возвращаем None.
fn parse_context_usage(usage: &Value) -> Option<ContextUsage> {
    let max_tokens = usage.get("max_input_tokens").and_then(Value::as_u64)?;
    if max_tokens == 0 {
        return None;
    }
    let used_tokens = usage
        .pointer("/actual/input_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("estimated_input_tokens").and_then(Value::as_u64))
        .unwrap_or(0);
    let compaction_trigger_tokens = usage
        .get("compaction_trigger_tokens")
        .and_then(Value::as_u64)
        .map(|tokens| tokens.min(u64::from(u32::MAX)) as u32);
    Some(ContextUsage {
        used_tokens: used_tokens.min(u64::from(u32::MAX)) as u32,
        max_tokens: max_tokens.min(u64::from(u32::MAX)) as u32,
        compaction_trigger_tokens,
    })
}

fn trim_tool_activities(items: &mut Vec<ToolActivity>) {
    let overflow = items.len().saturating_sub(12);
    if overflow > 0 {
        items.drain(0..overflow);
    }
}

fn tool_result_text(result: &Value) -> String {
    if let Some(error) = result.get("error").and_then(Value::as_str)
        && !error.is_empty()
    {
        return error.to_owned();
    }

    if let Some(output) = result.get("output").and_then(Value::as_str)
        && !output.is_empty()
    {
        return output.to_owned();
    }

    if let Some(content) = result.get("content")
        && !content.as_array().is_none_or(Vec::is_empty)
    {
        return tool_content_text(content);
    }

    if result.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        "(нет вывода)".to_owned()
    } else {
        "(tool завершился без текста ошибки)".to_owned()
    }
}

fn tool_content_text(content: &Value) -> String {
    let Some(parts) = content.as_array() else {
        return format_json(content);
    };

    let rendered = parts
        .iter()
        .filter_map(tool_content_part_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        format_json(content)
    } else {
        rendered.join("\n")
    }
}

fn tool_content_part_text(part: &Value) -> Option<String> {
    match part.get("type").and_then(Value::as_str) {
        Some("text") => part
            .get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        Some("json") => part.get("value").map(format_json),
        Some("image") => {
            let mime_type = part
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(format!("[image tool content: {mime_type}]"))
        }
        Some("binary") => {
            let mime_type = part
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(format!("[binary tool content: {mime_type}]"))
        }
        _ => Some(format_json(part)),
    }
}

fn compaction_report_text(report: &Value) -> String {
    let input_messages = report
        .get("input_messages")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_messages = report
        .get("output_messages")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let original_tokens = report
        .get("original_token_estimate")
        .and_then(Value::as_u64);
    let output_tokens = report.get("output_token_estimate").and_then(Value::as_u64);
    let summary_source = report
        .get("summary_source")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    let token_text = match (original_tokens, output_tokens) {
        (Some(before), Some(after)) => format!(", ~{before} -> ~{after} tokens"),
        _ => String::new(),
    };
    format!(
        "История сжата: {input_messages} -> {output_messages} сообщений{token_text}. Summary: {summary_source}."
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tool_result_text_keeps_full_output_for_line_preview() {
        let output = "x".repeat(700);
        let result = json!({
            "call_id": "call-1",
            "ok": true,
            "output": output,
            "content": [],
            "error": null,
        });

        assert_eq!(tool_result_text(&result), "x".repeat(700));
    }

    #[test]
    fn tool_content_text_renders_all_structured_parts_without_compacting_list() {
        let content = json!([
            {"type": "text", "text": "first"},
            {"type": "json", "value": {"a": 1, "b": [2, 3]}},
            {"type": "text", "text": "third"},
            {"type": "text", "text": "fourth"},
            {"type": "text", "text": "fifth"}
        ]);

        let rendered = tool_content_text(&content);

        assert!(rendered.contains("first"));
        assert!(rendered.contains("\"b\": ["));
        assert!(rendered.contains("fifth"));
        assert!(!rendered.contains("..."));
    }
}
