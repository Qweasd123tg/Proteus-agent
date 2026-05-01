use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{MemoryPolicy, MemoryPolicyInput, MemoryPolicyOutput, MemoryStore},
    domain::MemoryItem,
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

#[derive(Debug)]
pub struct NoMemoryPolicy;

#[async_trait]
impl MemoryPolicy for NoMemoryPolicy {
    async fn after_turn(
        &self,
        _input: MemoryPolicyInput<'_>,
        _memory: &dyn MemoryStore,
    ) -> Result<MemoryPolicyOutput> {
        Ok(MemoryPolicyOutput::default())
    }
}

/// Heuristic policy: после каждого turn'а пишет ровно один `MemoryItem`
/// с `kind = "carry_forward:latest"` — последнее assistant-сообщение
/// turn'а, сжатое до 500 символов. Это "handoff note" для следующего
/// turn'а / сессии: короткая выжимка что только что произошло.
///
/// Пишет только если в `new_messages` есть assistant-сообщение с
/// непустым текстом. Тool-only turn'ы (только tool_call без текста) не
/// трекаются.
///
/// Старые carry_forward items **остаются в базе** (append) — MemoryStore
/// отвечает за их выпадение через TTL/GC. `recall` по kind'у достаёт
/// свежайший через `ORDER BY id DESC`.
#[derive(Debug)]
pub struct CarryForwardPolicy;

/// Максимальная длина content для `carry_forward:latest`. Не ограничиваем
/// жёстко, но и не тащим весь многостраничный ответ — handoff-note
/// должна быть короткой.
const CARRY_FORWARD_CONTENT_LIMIT: usize = 500;

/// Имя `kind`-а для carry-forward. Вынесено в константу чтобы tests и
/// будущий recall-код ссылались на одну строку.
pub const CARRY_FORWARD_KIND: &str = "carry_forward:latest";

#[async_trait]
impl MemoryPolicy for CarryForwardPolicy {
    async fn after_turn(
        &self,
        input: MemoryPolicyInput<'_>,
        memory: &dyn MemoryStore,
    ) -> Result<MemoryPolicyOutput> {
        let Some(text) = extract_latest_assistant_text(input.new_messages) else {
            return Ok(MemoryPolicyOutput::default());
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(MemoryPolicyOutput::default());
        }
        let snippet: String = trimmed.chars().take(CARRY_FORWARD_CONTENT_LIMIT).collect();
        let item = MemoryItem::new(CARRY_FORWARD_KIND, snippet, serde_json::Value::Null);
        memory.remember(item).await?;
        Ok(MemoryPolicyOutput {
            written_kinds: vec![CARRY_FORWARD_KIND.to_string()],
        })
    }
}

/// Возвращает склеенный текст последнего assistant-сообщения с непустыми
/// `Text`-частями. Идём с конца `messages`, останавливаемся на первом
/// подходящем. Сообщения без текстовых частей (pure tool_call) пропускаем.
fn extract_latest_assistant_text(messages: &[CanonicalMessage]) -> Option<String> {
    for message in messages.iter().rev() {
        if message.role != MessageRole::Assistant {
            continue;
        }
        let joined = message
            .parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.trim().is_empty() {
            return Some(joined);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::MemoryPolicy,
        domain::{AgentOutput, AgentTask, MemoryItem, MemoryQuery},
    };
    use std::{path::PathBuf, sync::Arc};
    use tokio::sync::Mutex as AsyncMutex;

    #[derive(Default)]
    struct RecordingMemory {
        items: AsyncMutex<Vec<MemoryItem>>,
    }

    #[async_trait]
    impl MemoryStore for RecordingMemory {
        async fn remember(&self, item: MemoryItem) -> Result<()> {
            self.items.lock().await.push(item);
            Ok(())
        }
        async fn recall(&self, _q: MemoryQuery) -> Result<Vec<MemoryItem>> {
            Ok(self.items.lock().await.clone())
        }
    }

    fn assistant_message(text: &str) -> CanonicalMessage {
        CanonicalMessage::text(MessageRole::Assistant, text)
    }

    fn user_message(text: &str) -> CanonicalMessage {
        CanonicalMessage::text(MessageRole::User, text)
    }

    fn make_input<'a>(
        task: &'a AgentTask,
        output: &'a AgentOutput,
        messages: &'a [CanonicalMessage],
    ) -> MemoryPolicyInput<'a> {
        MemoryPolicyInput {
            task,
            output,
            new_messages: messages,
        }
    }

    #[test]
    fn extract_picks_last_assistant_text() {
        let messages = vec![
            user_message("hi"),
            assistant_message("first"),
            user_message("more"),
            assistant_message("final answer"),
        ];
        let got = extract_latest_assistant_text(&messages);
        assert_eq!(got.as_deref(), Some("final answer"));
    }

    #[test]
    fn extract_skips_assistant_without_text_parts() {
        let tool_only = CanonicalMessage::new(MessageRole::Assistant, vec![]);
        let messages = vec![assistant_message("earlier"), tool_only];
        let got = extract_latest_assistant_text(&messages);
        assert_eq!(got.as_deref(), Some("earlier"));
    }

    #[test]
    fn extract_returns_none_without_assistant_text() {
        let messages = vec![user_message("hello")];
        assert!(extract_latest_assistant_text(&messages).is_none());
    }

    #[tokio::test]
    async fn policy_writes_carry_forward_after_assistant_text() {
        let memory = Arc::new(RecordingMemory::default());
        let task = AgentTask::new("do thing", PathBuf::from("/tmp"));
        let output = AgentOutput::new("done", serde_json::Value::Null);
        let messages = vec![user_message("go"), assistant_message("all done")];

        let policy = CarryForwardPolicy;
        let result = policy
            .after_turn(make_input(&task, &output, &messages), memory.as_ref())
            .await
            .unwrap();

        assert_eq!(result.written_kinds, vec![CARRY_FORWARD_KIND.to_string()]);
        let items = memory.items.lock().await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, CARRY_FORWARD_KIND);
        assert_eq!(items[0].content, "all done");
    }

    #[tokio::test]
    async fn policy_is_noop_without_assistant_text() {
        let memory = Arc::new(RecordingMemory::default());
        let task = AgentTask::new("x", PathBuf::from("/tmp"));
        let output = AgentOutput::new("x", serde_json::Value::Null);
        let messages = vec![user_message("just user")];
        let policy = CarryForwardPolicy;
        let result = policy
            .after_turn(make_input(&task, &output, &messages), memory.as_ref())
            .await
            .unwrap();
        assert!(result.written_kinds.is_empty());
        assert!(memory.items.lock().await.is_empty());
    }

    #[tokio::test]
    async fn policy_truncates_long_content() {
        let long = "x".repeat(1000);
        let memory = Arc::new(RecordingMemory::default());
        let task = AgentTask::new("y", PathBuf::from("/tmp"));
        let output = AgentOutput::new("y", serde_json::Value::Null);
        let messages = vec![assistant_message(&long)];
        let policy = CarryForwardPolicy;
        policy
            .after_turn(make_input(&task, &output, &messages), memory.as_ref())
            .await
            .unwrap();
        let items = memory.items.lock().await;
        assert_eq!(
            items[0].content.chars().count(),
            CARRY_FORWARD_CONTENT_LIMIT
        );
    }
}
