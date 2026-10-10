use proteus_contracts::{
    domain::{MemoryItem, MemoryQuery, ToolCall, ToolResult, ToolSafety, ToolSpec},
    process_module::{
        ProcessModuleError, ProcessModuleResult, ToolModule, ToolModuleHost,
        ToolModuleInvocationContext,
    },
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct MemoryTool {
    pub store: Arc<super::SqliteMemoryStore>,
    pub remember: bool,
}

impl ToolModule for MemoryTool {
    fn spec_json(&self) -> String {
        let spec = if self.remember {
            ToolSpec::new(
                "remember_fact",
                "Store a durable fact in long-term memory. Use for stable user/team preferences and codebase invariants that should survive across turns and sessions. Do not use for transient progress notes.",
                json!({
                    "type": "object", "properties": {
                        "kind": {"type": "string", "enum": ["preference", "fact"], "description": "preference = user/team conventions; fact = codebase invariants, API contracts, architectural decisions"},
                        "content": {"type": "string", "description": "The fact itself, short and self-contained. Avoid chat-style context."},
                        "metadata": {"type": "object", "description": "Optional structured context (source, scope, tags)."}
                    }, "required": ["kind", "content"]
                }),
                ToolSafety::WritesFiles,
            )
        } else {
            ToolSpec::new("recall_memory", "Recall durable preferences and facts from long-term memory.", json!({
                "type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}}, "required": ["query"]
            }), ToolSafety::ReadOnly).with_parallel_tool_calls(true)
        };
        serde_json::to_string(&spec).expect("memory tool spec")
    }

    fn invoke_json(
        &self,
        call_json: String,
        context_json: String,
        host: &mut dyn ToolModuleHost,
    ) -> ProcessModuleResult<String> {
        let call: ToolCall = serde_json::from_str(&call_json).map_err(error)?;
        let _: ToolModuleInvocationContext = serde_json::from_str(&context_json).map_err(error)?;
        if host.is_cancelled()? {
            return Err(ProcessModuleError::new("memory tool was canceled"));
        }
        let result = if self.remember {
            let kind = call
                .args
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| ProcessModuleError::new("remember_fact: missing 'kind'"))?;
            if !matches!(kind, "preference" | "fact") {
                return Err(ProcessModuleError::new(format!(
                    "remember_fact: 'kind' must be 'preference' or 'fact', got '{kind}'"
                )));
            }
            let content = call
                .args
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| ProcessModuleError::new("remember_fact: missing 'content'"))?;
            if content.trim().is_empty() {
                return Err(ProcessModuleError::new(
                    "remember_fact: 'content' must be non-empty",
                ));
            }
            self.store.remember(&MemoryItem::new(
                kind,
                content,
                call.args.get("metadata").cloned().unwrap_or(Value::Null),
            ))?;
            ToolResult::ok(call.id, format!("Remembered ({kind}): {content}"))
        } else {
            let query = call
                .args
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ProcessModuleError::new("recall_memory requires string arg 'query'")
                })?;
            let limit = call.args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let items = self.store.recall(&MemoryQuery::new(query, limit))?;
            let output = if items.is_empty() {
                "(no memories)".into()
            } else {
                items
                    .iter()
                    .map(|item| format!("{}: {}", item.kind, item.content))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            ToolResult::new(
                call.id,
                true,
                output,
                Vec::new(),
                None,
                json!({"items": items}),
            )
        };
        serde_json::to_string(&result).map_err(error)
    }
}

fn error(error: serde_json::Error) -> ProcessModuleError {
    ProcessModuleError::new(error.to_string())
}
