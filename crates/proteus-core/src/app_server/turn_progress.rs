use crate::domain::{Event, EventEnvelope, ToolCall};

use super::transcript::{AppTranscriptMessage, AppTranscriptSubagent, AppTranscriptTool};

/// Живой прогресс незавершённого хода: сегменты текста и tool-вызовы,
/// накопленные из stream-событий. History получает сообщения хода только при
/// его коммите в конце, а у SSE нет replay — клиент, открывший страницу
/// посреди хода, может восстановить уже настриманное только отсюда:
/// /history отдаёт эти сообщения хвостом после истории.
#[derive(Default)]
pub(super) struct TurnProgress {
    messages: Vec<AppTranscriptMessage>,
}

impl TurnProgress {
    /// Обновляет прогресс по runtime-событию. Вызывается из форвардера,
    /// который и так читает весь поток событий сессии.
    pub(super) fn apply(&mut self, envelope: &EventEnvelope) {
        let event = &envelope.event;
        match event {
            Event::TurnStarted { .. } => self.messages.clear(),
            Event::AssistantTextDelta { text } => self.append_text(text),
            Event::ToolCallRequested { call } => {
                self.append_tool_call(&envelope.thread_id.to_string(), call);
            }
            Event::ApprovalRequested { call_id, .. } => {
                self.set_tool_status(call_id, "waiting_approval", None);
            }
            Event::ApprovalResolved { call_id, approved } => {
                let status = if *approved { "approved" } else { "denied" };
                self.set_tool_status(call_id, status, None);
            }
            Event::ToolFinished { result } => {
                let status = if result.ok { "done" } else { "failed" };
                self.set_tool_status(&result.call_id, status, Some(result.text_or_status()));
            }
            Event::SubagentStarted {
                role,
                description,
                child_thread_id,
            } => self.messages.push(AppTranscriptMessage {
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
            }),
            Event::SubagentFinished {
                role: _,
                status,
                iterations,
                child_thread_id,
            } => self.set_subagent_status(&child_thread_id.to_string(), status, Some(*iterations)),
            // Ход закончился (успехом или ошибкой): его сообщения теперь либо
            // закоммичены в history, либо потеряны вместе с ходом — прогресс
            // не должен пережить ход и стать фантомом в /history.
            Event::TurnFinished { .. } | Event::Error { .. } => self.messages.clear(),
            _ => {}
        }
    }

    pub(super) fn clear(&mut self) {
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
        if let Some(subagent) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.subagent.as_mut())
            .find(|subagent| subagent.child_thread_id == thread_id && subagent.status == "running")
        {
            if !subagent.tools.iter().any(|tool| tool.call_id == call.id) {
                subagent.tools.push(AppTranscriptTool {
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    args: call.args.clone(),
                    status: "running".to_owned(),
                    result: None,
                });
            }
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
            }),
            subagent: None,
            streaming: false,
        });
    }

    fn set_tool_status(&mut self, call_id: &str, status: &str, result: Option<String>) {
        for message in self.messages.iter_mut().rev() {
            if let Some(tool) = message.tool.as_mut().filter(|tool| tool.call_id == call_id) {
                tool.status = status.to_owned();
                if let Some(result) = result {
                    tool.result = Some(result);
                }
                return;
            }
            if let Some(tool) = message.subagent.as_mut().and_then(|subagent| {
                subagent
                    .tools
                    .iter_mut()
                    .find(|tool| tool.call_id == call_id)
            }) {
                tool.status = status.to_owned();
                if let Some(result) = result {
                    tool.result = Some(result);
                }
                return;
            }
        }
    }

    fn set_subagent_status(
        &mut self,
        child_thread_id: &str,
        status: &str,
        iterations: Option<u32>,
    ) {
        if let Some(subagent) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.subagent.as_mut())
            .find(|subagent| subagent.child_thread_id == child_thread_id)
        {
            subagent.status = status.to_owned();
            subagent.iterations = iterations;
        }
    }
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
}
