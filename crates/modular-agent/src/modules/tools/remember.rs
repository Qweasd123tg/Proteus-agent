//! `remember_fact` tool: модель вызывает его чтобы явно положить
//! preference/fact в long-term memory.
//!
//! Владеет `Arc<dyn MemoryStore>` (тот же паттерн что `ApplyPatchTool`
//! для patch applier'а). Не требует расширения `ToolContext`.
//!
//! `kind` ограничен двумя значениями:
//! - `"preference"` — устойчивые user/team conventions ("prefer tabs",
//!   "use React Router v6");
//! - `"fact"` — codebase invariants, API contracts, architectural decisions.
//!
//! `"carry_forward:latest"` — policy-managed, через этот tool не пишется.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    contracts::{MemoryStore, Tool, ToolContext},
    domain::{MemoryItem, ToolCall, ToolResult, ToolSafety, ToolSpec},
};

#[derive(Clone)]
pub struct RememberFactTool {
    memory: Arc<dyn MemoryStore>,
}

impl RememberFactTool {
    pub fn new(memory: Arc<dyn MemoryStore>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for RememberFactTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "remember_fact",
            "Store a durable fact in long-term memory. Use for stable \
             user/team preferences and codebase invariants that should \
             survive across turns and sessions. Do not use for \
             transient progress notes.",
            json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["preference", "fact"],
                        "description": "preference = user/team conventions (e.g. \"prefer tabs\", \"use React Router v6\"); fact = codebase invariants, API contracts, architectural decisions"
                    },
                    "content": {
                        "type": "string",
                        "description": "The fact itself, short and self-contained. Avoid chat-style context."
                    },
                    "metadata": {
                        "type": "object",
                        "description": "Optional structured context (source, scope, tags)."
                    }
                },
                "required": ["kind", "content"]
            }),
            ToolSafety::WritesFiles,
        )
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> Result<ToolResult> {
        let kind = call
            .args
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("remember_fact: missing 'kind'"))?;
        if !matches!(kind, "preference" | "fact") {
            bail!("remember_fact: 'kind' must be 'preference' or 'fact', got '{kind}'");
        }
        let content = call
            .args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("remember_fact: missing 'content'"))?;
        if content.trim().is_empty() {
            bail!("remember_fact: 'content' must be non-empty");
        }
        let metadata = call
            .args
            .get("metadata")
            .cloned()
            .unwrap_or(Value::Null);
        self.memory
            .remember(MemoryItem::new(kind, content, metadata))
            .await?;
        Ok(ToolResult::ok(
            call.id.clone(),
            format!("Remembered ({kind}): {content}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::MemoryStore,
        domain::{MemoryQuery, new_call_id},
    };
    use std::sync::Mutex;
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
        async fn recall(&self, _query: MemoryQuery) -> Result<Vec<MemoryItem>> {
            Ok(self.items.lock().await.clone())
        }
    }

    fn make_call(args: Value) -> ToolCall {
        ToolCall::new(new_call_id(), "remember_fact", args)
    }

    fn ctx() -> ToolContext {
        ToolContext::new(std::path::PathBuf::from("/tmp"))
    }

    #[tokio::test]
    async fn remembers_preference() {
        let memory = Arc::new(RecordingMemory::default());
        let tool = RememberFactTool::new(memory.clone());
        let result = tool
            .invoke(
                &make_call(json!({
                    "kind": "preference",
                    "content": "user prefers tabs"
                })),
                ctx(),
            )
            .await
            .unwrap();
        assert!(result.ok);
        let items = memory.items.lock().await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "preference");
        assert_eq!(items[0].content, "user prefers tabs");
    }

    #[tokio::test]
    async fn remembers_fact_with_metadata() {
        let memory = Arc::new(RecordingMemory::default());
        let tool = RememberFactTool::new(memory.clone());
        tool.invoke(
            &make_call(json!({
                "kind": "fact",
                "content": "API /users returns {id, email}",
                "metadata": { "source": "docs" }
            })),
            ctx(),
        )
        .await
        .unwrap();
        let items = memory.items.lock().await;
        assert_eq!(items[0].kind, "fact");
        assert_eq!(items[0].metadata["source"], "docs");
    }

    #[tokio::test]
    async fn rejects_invalid_kind() {
        let memory = Arc::new(RecordingMemory::default());
        let tool = RememberFactTool::new(memory.clone());
        let err = tool
            .invoke(
                &make_call(json!({
                    "kind": "random",
                    "content": "anything"
                })),
                ctx(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("must be 'preference' or 'fact'"));
        assert!(memory.items.lock().await.is_empty());
    }

    #[tokio::test]
    async fn rejects_empty_content() {
        let memory = Arc::new(RecordingMemory::default());
        let tool = RememberFactTool::new(memory.clone());
        let err = tool
            .invoke(
                &make_call(json!({
                    "kind": "fact",
                    "content": "   "
                })),
                ctx(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("non-empty"));
    }

    #[tokio::test]
    async fn rejects_missing_kind() {
        let memory = Arc::new(RecordingMemory::default());
        let tool = RememberFactTool::new(memory.clone());
        let err = tool
            .invoke(&make_call(json!({ "content": "x" })), ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'kind'"));
    }

    // Проверяем что std::sync::Mutex импорт не помешал — он не используется
    // в тестах (только AsyncMutex), но оставлен на случай future-proof.
    #[test]
    fn std_mutex_import_compiles() {
        let _: Mutex<()> = Mutex::new(());
    }
}
