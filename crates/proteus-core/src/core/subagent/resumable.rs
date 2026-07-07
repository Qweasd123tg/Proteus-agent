//! In-memory store resumable-снапшотов дочерних циклов (`task_id` → history).
//!
//! Снапшот сохраняется после каждого завершения дочернего цикла, включая
//! `Cancelled` и `TimedOut`: прерванный ребёнок не должен терять частичную
//! работу — её можно продолжить через `task_id`. Перед сохранением история
//! санитизируется: незакрытые tool calls получают синтетический tool result,
//! чтобы resume-история оставалась валидной для provider-а.

use std::collections::{HashMap, HashSet};

use crate::{
    domain::{CallId, SessionId, ToolResult},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

#[derive(Debug, Default)]
pub(super) struct ResumableStore {
    snapshots: HashMap<String, ResumableSnapshot>,
    clock: u64,
}

#[derive(Debug, Clone)]
pub(super) struct ResumableSnapshot {
    pub session_id: SessionId,
    pub role_name: String,
    pub history: Vec<CanonicalMessage>,
    last_used: u64,
}

impl ResumableStore {
    pub fn get(&mut self, task_id: &str) -> Option<ResumableSnapshot> {
        self.clock = self.clock.saturating_add(1);
        let snapshot = self.snapshots.get_mut(task_id)?;
        snapshot.last_used = self.clock;
        Some(snapshot.clone())
    }

    pub fn save(
        &mut self,
        key: String,
        session_id: SessionId,
        role_name: String,
        history: Vec<CanonicalMessage>,
        max_resumable: usize,
    ) -> bool {
        if max_resumable == 0 {
            self.snapshots.remove(&key);
            return false;
        }

        self.clock = self.clock.saturating_add(1);
        self.snapshots.insert(
            key,
            ResumableSnapshot {
                session_id,
                role_name,
                history: close_dangling_tool_calls(history),
                last_used: self.clock,
            },
        );
        while self.snapshots.len() > max_resumable {
            let Some(evicted_key) = self
                .snapshots
                .iter()
                .min_by_key(|(_, snapshot)| snapshot.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.snapshots.remove(&evicted_key);
        }
        true
    }
}

/// Закрывает незакрытые tool calls синтетическими tool results.
///
/// Cancel/timeout может прервать дочерний цикл между assistant-сообщением с
/// tool calls и записью их результатов; история с «висящими» calls невалидна
/// для большинства provider API. Ответы добавляются в конец в порядке
/// появления calls (незакрытыми могут быть только calls последнего
/// assistant-сообщения — цикл пишет результаты сразу за каждым вызовом).
fn close_dangling_tool_calls(mut history: Vec<CanonicalMessage>) -> Vec<CanonicalMessage> {
    let answered: HashSet<CallId> = history
        .iter()
        .filter(|message| message.role == MessageRole::Tool)
        .flat_map(|message| {
            message
                .tool_call_id
                .clone()
                .into_iter()
                .chain(message.parts.iter().filter_map(|part| match part {
                    ContentPart::ToolResult { result } => Some(result.call_id.clone()),
                    _ => None,
                }))
        })
        .collect();

    let dangling: Vec<CallId> = history
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
        .flat_map(|message| {
            message.parts.iter().filter_map(|part| match part {
                ContentPart::ToolCall { call } if !answered.contains(&call.id) => {
                    Some(call.id.clone())
                }
                _ => None,
            })
        })
        .collect();

    for call_id in dangling {
        let result = ToolResult::error(
            call_id.clone(),
            "tool call was interrupted (subagent cancelled or timed out); the result is unavailable",
        );
        history.push(
            CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                .with_tool_call_id(call_id),
        );
    }
    history
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ToolCall;

    #[test]
    fn dangling_tool_calls_are_closed_with_synthetic_results() {
        let call = ToolCall::new("call-1", "shell", serde_json::json!({}));
        let answered_call = ToolCall::new("call-0", "read_file", serde_json::json!({}));
        let history = vec![
            CanonicalMessage::text(MessageRole::System, "prompt"),
            CanonicalMessage::text(MessageRole::User, "task"),
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall {
                    call: answered_call.clone(),
                }],
            ),
            CanonicalMessage::new(
                MessageRole::Tool,
                vec![ContentPart::ToolResult {
                    result: ToolResult::ok(answered_call.id.clone(), "ok"),
                }],
            )
            .with_tool_call_id(answered_call.id),
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            ),
        ];

        let closed = close_dangling_tool_calls(history);

        assert_eq!(closed.len(), 6);
        let synthetic = closed.last().expect("synthetic tool result");
        assert_eq!(synthetic.role, MessageRole::Tool);
        assert_eq!(synthetic.tool_call_id.as_deref(), Some(call.id.as_str()));
        match &synthetic.parts[0] {
            ContentPart::ToolResult { result } => {
                assert_eq!(result.call_id, call.id);
                assert!(!result.ok);
                assert!(
                    result
                        .error
                        .as_deref()
                        .is_some_and(|error| error.contains("interrupted"))
                );
            }
            other => panic!("unexpected part: {other:?}"),
        }
    }

    #[test]
    fn history_without_dangling_calls_is_unchanged() {
        let history = vec![
            CanonicalMessage::text(MessageRole::System, "prompt"),
            CanonicalMessage::text(MessageRole::User, "task"),
            CanonicalMessage::text(MessageRole::Assistant, "answer"),
        ];

        let closed = close_dangling_tool_calls(history.clone());

        assert_eq!(closed, history);
    }
}
