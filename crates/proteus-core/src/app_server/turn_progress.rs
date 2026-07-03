use crate::domain::Event;

use super::transcript::{AppTranscriptMessage, AppTranscriptTool};

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
    pub(super) fn apply(&mut self, event: &Event) {
        match event {
            Event::TurnStarted { .. } => self.messages.clear(),
            Event::AssistantTextDelta { text } => self.append_text(text),
            Event::ToolCallRequested { call } => self.messages.push(AppTranscriptMessage {
                role: "system".to_owned(),
                text: String::new(),
                tool: Some(AppTranscriptTool {
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    args: call.args.clone(),
                    status: "running".to_owned(),
                    result: None,
                }),
                streaming: false,
            }),
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
        {
            last.text.push_str(text);
            return;
        }
        self.messages.push(AppTranscriptMessage {
            role: "assistant".to_owned(),
            text: text.to_owned(),
            tool: None,
            streaming: false,
        });
    }

    fn set_tool_status(&mut self, call_id: &str, status: &str, result: Option<String>) {
        if let Some(tool) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.tool.as_mut())
            .find(|tool| tool.call_id == call_id)
        {
            tool.status = status.to_owned();
            if let Some(result) = result {
                tool.result = Some(result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::{ToolCall, ToolResult};

    fn delta(text: &str) -> Event {
        Event::AssistantTextDelta {
            text: text.to_owned(),
        }
    }

    #[test]
    fn accumulates_text_segments_around_tool_calls() {
        let mut progress = TurnProgress::default();
        progress.apply(&delta("Сначала "));
        progress.apply(&delta("посмотрю файл."));
        progress.apply(&Event::ToolCallRequested {
            call: ToolCall::new("call-1", "read_file", json!({ "path": "src/lib.rs" })),
        });
        progress.apply(&Event::ToolFinished {
            result: ToolResult::ok("call-1".to_owned(), "contents"),
        });
        progress.apply(&delta("Теперь answer."));

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
        progress.apply(&delta("Запускаю."));
        progress.apply(&Event::ToolCallRequested {
            call: ToolCall::new("call-1", "shell", json!({})),
        });

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
        progress.apply(&delta("старый ход"));
        progress.apply(&Event::TurnStarted {
            session_id: crate::domain::new_session_id(),
            thread_id: crate::domain::new_thread_id(),
            turn_id: crate::domain::new_turn_id(),
        });
        assert!(progress.snapshot().is_empty());

        progress.apply(&delta("новый ход"));
        progress.apply(&Event::Error {
            message: "boom".to_owned(),
        });
        assert!(progress.snapshot().is_empty());
    }
}
