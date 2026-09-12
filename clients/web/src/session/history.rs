use crate::{
    messages::adopt_streaming_tail,
    tool_names::{FOLLOWUP_TASK_TOOL, SPAWN_AGENT_TOOL, TASK_TOOL},
    types::*,
    ui_utils::{compact_text, format_json},
};
use leptos::prelude::*;
use serde_json::Value;

pub(crate) fn apply_transcript(
    items: Vec<TranscriptMessage>,
    set_messages: crate::transcript::TranscriptWriter,
    set_next_message_id: WriteSignal<u64>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
) {
    let transcript = transcript_messages(items);
    set_next_message_id.set(next_message_id_after(&transcript));
    set_active_stream_message_id.set(None);
    set_streamed_this_turn.set(false);
    adopt_streaming_tail(
        &transcript,
        set_active_stream_message_id,
        set_streamed_this_turn,
    );
    set_messages.set(transcript);
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
                message_id: item.message_id.map(|id| id.to_string()),
                phase: item.phase,
                id: 0,
                version: 0,
                text_offset: 0,
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
            message_id: item.message_id.map(|id| id.to_string()),
            phase: item.phase,
            id: 0,
            version: 0,
            text_offset: 0,
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
        SPAWN_AGENT_TOOL => tool.args.get("task_name").and_then(Value::as_str) == Some(task_name),
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

#[cfg(test)]
mod tests;
