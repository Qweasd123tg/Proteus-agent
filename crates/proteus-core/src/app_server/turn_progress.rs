use crate::domain::{Event, EventEnvelope, SteeringDeliveryKind, ThreadId, ToolCall};

use super::transcript::{AppTranscriptMessage, AppTranscriptSubagent, AppTranscriptTool};

const MAX_BACKGROUND_SUBAGENTS: usize = 64;
const MAX_SUBAGENT_TOOLS: usize = 64;
const MAX_SUBAGENT_TOOL_RESULT_BYTES: usize = 10_000;
const MAX_BACKGROUND_TOOL_JSON_BYTES: usize = 8_000;

/// Живой прогресс незавершённого хода: сегменты текста и tool-вызовы,
/// накопленные из stream-событий. History получает сообщения хода только при
/// его коммите в конце, а у SSE нет replay — клиент, открывший страницу
/// посреди хода, может восстановить уже настриманное только отсюда:
/// /history отдаёт эти сообщения хвостом после истории.
#[derive(Default)]
pub(super) struct TurnProgress {
    messages: Vec<AppTranscriptMessage>,
    /// Collaboration-дети, запущенные через `spawn_agent`/`followup_task`,
    /// живут дольше родительского turn-а. Их карточки хранятся отдельно, чтобы
    /// TurnFinished/следующий TurnStarted не превращали поздние child events
    /// в плоские фантомные tool-карточки.
    background_subagents: Vec<AppTranscriptMessage>,
    /// Thread бегущего хода (из envelope TurnStarted). Text-дельты других
    /// threads (например, стрим дочернего цикла субагента из module
    /// runner-а) не подмешиваются в родительский текст.
    turn_thread_id: Option<ThreadId>,
}

impl TurnProgress {
    /// Обновляет прогресс по runtime-событию. Вызывается из форвардера,
    /// который и так читает весь поток событий сессии.
    pub(super) fn apply(&mut self, envelope: &EventEnvelope) {
        let event = &envelope.event;
        match event {
            Event::TurnStarted { .. } => {
                self.messages.clear();
                self.turn_thread_id = Some(envelope.thread_id);
            }
            Event::AssistantTextDelta {
                message_id,
                phase,
                text,
                ..
            } if self
                .turn_thread_id
                .is_none_or(|thread_id| thread_id == envelope.thread_id) =>
            {
                self.update_text(*message_id, *phase, text, false);
            }
            Event::AssistantTextDelta { .. } => {}
            Event::AssistantMessageCompleted {
                message_id,
                phase,
                text,
            } if self
                .turn_thread_id
                .is_none_or(|id| id == envelope.thread_id) =>
            {
                self.update_text(*message_id, *phase, text, true);
            }
            Event::AssistantMessageCompleted { .. } => {}
            Event::SteeringDelivered {
                text,
                kind: SteeringDeliveryKind::Steering,
                ..
            } => {
                self.messages.push(AppTranscriptMessage {
                    message_id: None,
                    phase: None,
                    role: "user".to_owned(),
                    text: text.clone(),
                    tool: None,
                    subagent: None,
                    streaming: false,
                });
            }
            Event::SteeringDelivered { .. } => {}
            Event::ToolCallRequested { call } => {
                self.append_tool_call(&envelope.thread_id.to_string(), call);
            }
            Event::ApprovalRequested { call_id, .. } => {
                self.set_tool_status(call_id, "waiting_approval", None, None);
            }
            Event::ApprovalResolved { call_id, approved } => {
                let status = if *approved { "approved" } else { "denied" };
                self.set_tool_status(call_id, status, None, None);
            }
            Event::ToolFinished { result } => {
                let status = if result.ok { "done" } else { "failed" };
                self.set_tool_status(
                    &result.call_id,
                    status,
                    Some(result.text_or_status()),
                    Some(&result.metadata),
                );
            }
            Event::SubagentStarted {
                role,
                description,
                child_thread_id,
            } => {
                let message = AppTranscriptMessage {
                    message_id: None,
                    phase: None,
                    role: "system".to_owned(),
                    text: String::new(),
                    tool: None,
                    subagent: Some(AppTranscriptSubagent {
                        child_thread_id: child_thread_id.to_string(),
                        role: role.clone(),
                        description: description.clone(),
                        status: "running".to_owned(),
                        iterations: None,
                        tools: Vec::new(),
                    }),
                    streaming: false,
                };
                if self.background_agent_parent_is_running(description.as_deref()) {
                    self.push_background_subagent(message);
                } else {
                    self.messages.push(message);
                }
            }
            Event::SubagentFinished {
                role: _,
                status,
                iterations,
                child_thread_id,
            } => {
                let child_thread_id = child_thread_id.to_string();
                if !set_subagent_status_in(
                    &mut self.messages,
                    &child_thread_id,
                    status,
                    Some(*iterations),
                ) {
                    set_subagent_status_in(
                        &mut self.background_subagents,
                        &child_thread_id,
                        status,
                        Some(*iterations),
                    );
                }
            }
            // Ход закончился (успехом или ошибкой): его сообщения теперь либо
            // закоммичены в history, либо потеряны вместе с ходом — прогресс
            // не должен пережить ход и стать фантомом в /history.
            Event::TurnFinished { .. } | Event::Error { .. } => self.messages.clear(),
            _ => {}
        }
    }

    pub(super) fn finish_parent_turn(&mut self) {
        self.messages.clear();
    }

    /// Снимок прогресса для /history с теми же item ids и состоянием завершения.
    pub(super) fn snapshot(&self) -> Vec<AppTranscriptMessage> {
        let mut messages = self.messages.clone();
        messages.extend(self.background_subagents.clone());
        messages
    }

    fn update_text(
        &mut self,
        message_id: crate::domain::MessageId,
        phase: Option<crate::model_standard::MessagePhase>,
        text: &str,
        completed: bool,
    ) {
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.message_id == Some(message_id))
        {
            if completed {
                message.text = text.to_owned();
            } else {
                message.text.push_str(text);
            }
            message.phase = if completed {
                phase
            } else {
                phase.or(message.phase)
            };
            message.streaming = !completed;
            return;
        }
        if text.is_empty() {
            return;
        }
        for message in &mut self.messages {
            message.streaming = false;
        }
        self.messages.push(AppTranscriptMessage {
            message_id: Some(message_id),
            phase,
            role: "assistant".to_owned(),
            text: text.to_owned(),
            tool: None,
            subagent: None,
            streaming: !completed,
        });
    }

    fn append_tool_call(&mut self, thread_id: &str, call: &ToolCall) {
        if append_subagent_tool(&mut self.background_subagents, thread_id, call, true)
            || append_subagent_tool(&mut self.messages, thread_id, call, false)
        {
            return;
        }

        for message in &mut self.messages {
            message.streaming = false;
        }
        self.messages.push(AppTranscriptMessage {
            message_id: None,
            phase: None,
            role: "system".to_owned(),
            text: String::new(),
            tool: Some(AppTranscriptTool {
                call_id: call.id.clone(),
                name: call.name.clone(),
                args: call.args.clone(),
                status: "running".to_owned(),
                result: None,
                metadata: serde_json::Value::Null,
            }),
            subagent: None,
            streaming: false,
        });
    }

    fn set_tool_status(
        &mut self,
        call_id: &str,
        status: &str,
        result: Option<String>,
        metadata: Option<&serde_json::Value>,
    ) {
        if set_tool_status_in(
            &mut self.messages,
            call_id,
            status,
            result.as_deref(),
            metadata,
            false,
        ) {
            return;
        }
        set_tool_status_in(
            &mut self.background_subagents,
            call_id,
            status,
            result.as_deref(),
            metadata,
            true,
        );
    }

    fn background_agent_parent_is_running(&self, description: Option<&str>) -> bool {
        self.messages.iter().rev().any(|message| {
            message.tool.as_ref().is_some_and(|tool| {
                tool.status == "running"
                    && match tool.name.as_str() {
                        "spawn_agent" => description.is_none_or(|task_name| {
                            tool.args
                                .get("task_name")
                                .and_then(serde_json::Value::as_str)
                                == Some(task_name)
                        }),
                        "followup_task" => description.is_none_or(|task_name| {
                            tool.args
                                .get("target")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|target| {
                                    target == task_name || target == format!("/root/{task_name}")
                                })
                        }),
                        _ => false,
                    }
            })
        })
    }

    fn push_background_subagent(&mut self, message: AppTranscriptMessage) {
        while self.background_subagents.len() >= MAX_BACKGROUND_SUBAGENTS {
            let Some(index) = self.background_subagents.iter().position(|message| {
                message
                    .subagent
                    .as_ref()
                    .is_none_or(|subagent| subagent.status != "running")
            }) else {
                // Control plane не допускает больше 64 одновременно активных
                // детей. Если события всё же разошлись, сохраняем уже
                // отслеживаемые карточки вместо неограниченного роста.
                return;
            };
            self.background_subagents.remove(index);
        }
        self.background_subagents.push(message);
    }
}

fn append_subagent_tool(
    messages: &mut [AppTranscriptMessage],
    thread_id: &str,
    call: &ToolCall,
    compact: bool,
) -> bool {
    let Some(subagent) = messages
        .iter_mut()
        .rev()
        .filter_map(|message| message.subagent.as_mut())
        .find(|subagent| subagent.child_thread_id == thread_id && subagent.status == "running")
    else {
        return false;
    };
    if !subagent.tools.iter().any(|tool| tool.call_id == call.id) {
        if subagent.tools.len() >= MAX_SUBAGENT_TOOLS {
            let index = subagent
                .tools
                .iter()
                .position(|tool| tool.status != "running")
                .unwrap_or(0);
            subagent.tools.remove(index);
        }
        subagent.tools.push(AppTranscriptTool {
            call_id: call.id.clone(),
            name: call.name.clone(),
            args: if compact {
                compact_json(&call.args, MAX_BACKGROUND_TOOL_JSON_BYTES)
            } else {
                call.args.clone()
            },
            status: "running".to_owned(),
            result: None,
            metadata: serde_json::Value::Null,
        });
    }
    true
}

fn set_tool_status_in(
    messages: &mut [AppTranscriptMessage],
    call_id: &str,
    status: &str,
    result: Option<&str>,
    metadata: Option<&serde_json::Value>,
    compact: bool,
) -> bool {
    for message in messages.iter_mut().rev() {
        if let Some(tool) = message.tool.as_mut().filter(|tool| tool.call_id == call_id) {
            update_tool(tool, status, result, metadata, compact);
            return true;
        }
        if let Some(tool) = message.subagent.as_mut().and_then(|subagent| {
            subagent
                .tools
                .iter_mut()
                .find(|tool| tool.call_id == call_id)
        }) {
            update_tool(tool, status, result, metadata, compact);
            return true;
        }
    }
    false
}

fn update_tool(
    tool: &mut AppTranscriptTool,
    status: &str,
    result: Option<&str>,
    metadata: Option<&serde_json::Value>,
    compact: bool,
) {
    tool.status = status.to_owned();
    if let Some(result) = result {
        tool.result = Some(truncate_utf8(
            result.to_owned(),
            MAX_SUBAGENT_TOOL_RESULT_BYTES,
        ));
    }
    if let Some(metadata) = metadata {
        tool.metadata = if compact {
            compact_json(metadata, MAX_BACKGROUND_TOOL_JSON_BYTES)
        } else {
            metadata.clone()
        };
    }
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

fn compact_json(value: &serde_json::Value, max_bytes: usize) -> serde_json::Value {
    let encoded = value.to_string();
    if encoded.len() <= max_bytes {
        return value.clone();
    }
    serde_json::json!({
        "truncated": true,
        "preview": truncate_utf8(encoded, max_bytes),
    })
}

fn set_subagent_status_in(
    messages: &mut [AppTranscriptMessage],
    child_thread_id: &str,
    status: &str,
    iterations: Option<u32>,
) -> bool {
    if let Some(subagent) = messages
        .iter_mut()
        .rev()
        .filter_map(|message| message.subagent.as_mut())
        .find(|subagent| subagent.child_thread_id == child_thread_id)
    {
        subagent.status = status.to_owned();
        subagent.iterations = iterations;
        return true;
    }
    false
}

#[cfg(test)]
mod tests;
