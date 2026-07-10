use crate::domain::{Event, EventEnvelope, ThreadId, ToolCall};

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
    /// Collaboration-дети, запущенные через `spawn_agent`, живут дольше
    /// родительского turn-а. Их карточки хранятся отдельно, чтобы
    /// TurnFinished/следующий TurnStarted не превращали поздние child events
    /// в плоские фантомные tool-карточки.
    background_subagents: Vec<AppTranscriptMessage>,
    /// Thread бегущего хода (из envelope TurnStarted). Text-дельты других
    /// threads (например, стрим дочернего цикла субагента из плагинного
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
            Event::AssistantTextDelta { text }
                if self
                    .turn_thread_id
                    .is_none_or(|thread_id| thread_id == envelope.thread_id) =>
            {
                self.append_text(text);
            }
            Event::AssistantTextDelta { .. } => {}
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
                if self.spawn_agent_is_running(description.as_deref()) {
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

    /// Снимок прогресса для /history. Последний текстовый сегмент помечается
    /// streaming — клиент делает его целью для последующих SSE-дельт.
    pub(super) fn snapshot(&self) -> Vec<AppTranscriptMessage> {
        let mut messages = self.messages.clone();
        if let Some(last) = messages.last_mut()
            && last.tool.is_none()
            && last.subagent.is_none()
        {
            last.streaming = true;
        }
        messages.extend(self.background_subagents.clone());
        messages
    }

    fn append_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.messages.last_mut()
            && last.tool.is_none()
            && last.subagent.is_none()
        {
            last.text.push_str(text);
            return;
        }
        self.messages.push(AppTranscriptMessage {
            role: "assistant".to_owned(),
            text: text.to_owned(),
            tool: None,
            subagent: None,
            streaming: false,
        });
    }

    fn append_tool_call(&mut self, thread_id: &str, call: &ToolCall) {
        if append_subagent_tool(&mut self.background_subagents, thread_id, call, true)
            || append_subagent_tool(&mut self.messages, thread_id, call, false)
        {
            return;
        }

        self.messages.push(AppTranscriptMessage {
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

    fn spawn_agent_is_running(&self, description: Option<&str>) -> bool {
        self.messages.iter().rev().any(|message| {
            message.tool.as_ref().is_some_and(|tool| {
                tool.name == "spawn_agent"
                    && tool.status == "running"
                    && description.is_none_or(|task_name| {
                        tool.args
                            .get("task_name")
                            .and_then(serde_json::Value::as_str)
                            == Some(task_name)
                    })
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
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::{
        EventContext, ThreadId, ToolResult, new_session_id, new_thread_id, new_turn_id,
    };

    fn envelope(thread_id: ThreadId, event: Event) -> EventEnvelope {
        EventEnvelope::new(
            EventContext::new(new_session_id(), thread_id, Some(new_turn_id())),
            1,
            event,
        )
    }

    fn root_thread_id() -> ThreadId {
        ThreadId::parse_str("00000000-0000-0000-0000-000000000001").expect("root thread id")
    }

    fn child_thread_id() -> ThreadId {
        ThreadId::parse_str("00000000-0000-0000-0000-000000000002").expect("child thread id")
    }

    fn apply(progress: &mut TurnProgress, event: Event) {
        progress.apply(&envelope(root_thread_id(), event));
    }

    fn delta(text: &str) -> Event {
        Event::AssistantTextDelta {
            text: text.to_owned(),
        }
    }

    #[test]
    fn accumulates_text_segments_around_tool_calls() {
        let mut progress = TurnProgress::default();
        apply(&mut progress, delta("Сначала "));
        apply(&mut progress, delta("посмотрю файл."));
        apply(
            &mut progress,
            Event::ToolCallRequested {
                call: ToolCall::new("call-1", "read_file", json!({ "path": "src/lib.rs" })),
            },
        );
        apply(
            &mut progress,
            Event::ToolFinished {
                result: ToolResult::ok("call-1".to_owned(), "contents"),
            },
        );
        apply(&mut progress, delta("Теперь answer."));

        let snapshot = progress.snapshot();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].text, "Сначала посмотрю файл.");
        assert!(!snapshot[0].streaming);
        let tool = snapshot[1].tool.as_ref().expect("tool entry");
        assert_eq!(tool.status, "done");
        assert_eq!(tool.result.as_deref(), Some("contents"));
        // Последний текстовый сегмент — живой, клиент достримит в него.
        assert_eq!(snapshot[2].text, "Теперь answer.");
        assert!(snapshot[2].streaming);
    }

    #[test]
    fn trailing_tool_is_not_marked_streaming() {
        let mut progress = TurnProgress::default();
        apply(&mut progress, delta("Запускаю."));
        apply(
            &mut progress,
            Event::ToolCallRequested {
                call: ToolCall::new("call-1", "shell", json!({})),
            },
        );

        let snapshot = progress.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert!(!snapshot[1].streaming);
        assert_eq!(
            snapshot[1].tool.as_ref().map(|tool| tool.status.as_str()),
            Some("running")
        );
    }

    #[test]
    fn child_thread_text_deltas_do_not_pollute_parent_progress() {
        let mut progress = TurnProgress::default();
        apply(
            &mut progress,
            Event::TurnStarted {
                session_id: new_session_id(),
                thread_id: root_thread_id(),
                turn_id: new_turn_id(),
            },
        );
        apply(&mut progress, delta("родительский текст"));
        // Стрим дочернего цикла под child thread не должен доклеиваться к
        // родительскому сегменту (и не должен создавать свой).
        progress.apply(&envelope(
            child_thread_id(),
            Event::AssistantTextDelta {
                text: "детский стрим".to_owned(),
            },
        ));

        let snapshot = progress.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].text, "родительский текст");
    }

    #[test]
    fn turn_boundaries_clear_progress() {
        let mut progress = TurnProgress::default();
        apply(&mut progress, delta("старый ход"));
        apply(
            &mut progress,
            Event::TurnStarted {
                session_id: new_session_id(),
                thread_id: new_thread_id(),
                turn_id: new_turn_id(),
            },
        );
        assert!(progress.snapshot().is_empty());

        apply(&mut progress, delta("новый ход"));
        apply(
            &mut progress,
            Event::Error {
                message: "boom".to_owned(),
            },
        );
        assert!(progress.snapshot().is_empty());
    }

    #[test]
    fn snapshots_subagent_card_and_nested_child_tools() {
        let mut progress = TurnProgress::default();
        let child_thread_id = child_thread_id();

        apply(
            &mut progress,
            Event::SubagentStarted {
                role: "reviewer".to_owned(),
                description: Some("check patch".to_owned()),
                child_thread_id,
            },
        );
        progress.apply(&envelope(
            child_thread_id,
            Event::ToolCallRequested {
                call: ToolCall::new("call-child", "read_file", json!({ "path": "src/lib.rs" })),
            },
        ));
        progress.apply(&envelope(
            child_thread_id,
            Event::ToolFinished {
                result: ToolResult::ok("call-child".to_owned(), "contents"),
            },
        ));
        apply(
            &mut progress,
            Event::SubagentFinished {
                role: "reviewer".to_owned(),
                status: "completed".to_owned(),
                iterations: 2,
                child_thread_id,
            },
        );

        let snapshot = progress.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert!(!snapshot[0].streaming);
        assert!(snapshot[0].tool.is_none());
        let subagent = snapshot[0].subagent.as_ref().expect("subagent");
        assert_eq!(subagent.role, "reviewer");
        assert_eq!(subagent.description.as_deref(), Some("check patch"));
        assert_eq!(subagent.child_thread_id, child_thread_id.to_string());
        assert_eq!(subagent.status, "completed");
        assert_eq!(subagent.iterations, Some(2));
        assert_eq!(subagent.tools.len(), 1);
        assert_eq!(subagent.tools[0].call_id, "call-child");
        assert_eq!(subagent.tools[0].status, "done");
        assert_eq!(subagent.tools[0].result.as_deref(), Some("contents"));
    }

    #[test]
    fn collaboration_subagent_survives_parent_turn_and_keeps_late_tools_nested() {
        let mut progress = TurnProgress::default();
        let child_thread_id = child_thread_id();

        apply(
            &mut progress,
            Event::ToolCallRequested {
                call: ToolCall::new(
                    "spawn-1",
                    "spawn_agent",
                    json!({
                        "task_name": "scan",
                        "message": "inspect",
                        "agent_type": "explore"
                    }),
                ),
            },
        );
        apply(
            &mut progress,
            Event::SubagentStarted {
                role: "explore".to_owned(),
                description: Some("scan".to_owned()),
                child_thread_id,
            },
        );
        apply(
            &mut progress,
            Event::ToolFinished {
                result: ToolResult::ok("spawn-1".to_owned(), "started"),
            },
        );
        apply(
            &mut progress,
            Event::TurnFinished {
                output: crate::domain::AgentOutput::text("spawned"),
            },
        );

        progress.apply(&envelope(
            child_thread_id,
            Event::ToolCallRequested {
                call: ToolCall::new("child-1", "read_file", json!({ "path": "src/lib.rs" })),
            },
        ));
        progress.apply(&envelope(
            child_thread_id,
            Event::ToolFinished {
                result: ToolResult::ok("child-1".to_owned(), "contents"),
            },
        ));

        let snapshot = progress.snapshot();
        assert_eq!(
            snapshot.len(),
            1,
            "late child tool must not become flat progress"
        );
        let subagent = snapshot[0].subagent.as_ref().expect("background subagent");
        assert_eq!(subagent.status, "running");
        assert_eq!(subagent.tools.len(), 1);
        assert_eq!(subagent.tools[0].status, "done");

        apply(
            &mut progress,
            Event::TurnStarted {
                session_id: new_session_id(),
                thread_id: root_thread_id(),
                turn_id: new_turn_id(),
            },
        );
        apply(&mut progress, delta("new parent turn"));
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.len(), 2, "next turn retains card");
        assert!(
            snapshot[0].streaming,
            "background card must not hide stream tail"
        );
        assert!(snapshot[1].subagent.is_some());

        apply(
            &mut progress,
            Event::SubagentFinished {
                role: "explore".to_owned(),
                status: "completed".to_owned(),
                iterations: 2,
                child_thread_id,
            },
        );
        let snapshot = progress.snapshot();
        assert_eq!(
            snapshot[1]
                .subagent
                .as_ref()
                .map(|subagent| subagent.status.as_str()),
            Some("completed")
        );
    }
}
