use proteus_contracts::{
    domain::{ContextChunk, SearchQuery, ToolCall, ToolResult, ToolSafety, ToolSpec},
    process_module::{
        ProcessModuleError, ProcessModuleResult, ToolModule, ToolModuleHost,
        ToolModuleInvocationContext,
    },
};
use serde_json::{Value, json};

pub(crate) struct SearchTool;

impl ToolModule for SearchTool {
    fn spec_json(&self) -> String {
        serde_json::to_string(&ToolSpec::new(
            "search",
            "Search the current workspace with ripgrep. Use grep for raw regex line search and search for filtered workspace search.",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Text or regex-like query to search for."},
                    "max_results": {"type": "integer"},
                    "use_case": {"type": "string"},
                    "starts_with": {"type": "array", "items": {"type": "string"}},
                    "ends_with": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["query"]
            }),
            ToolSafety::ReadOnly,
        ).with_parallel_tool_calls(true).with_timeout(60_000).with_metadata(json!({
            "hot": true, "category": "search", "tags": ["workspace", "search", "repo", "code"],
            "aliases": ["search_text", "ripgrep", "find text", "search code"]
        }))).expect("search spec")
    }

    fn invoke_json(
        &self,
        call_json: String,
        context_json: String,
        host: &mut dyn ToolModuleHost,
    ) -> ProcessModuleResult<String> {
        let call: ToolCall = serde_json::from_str(&call_json).map_err(error)?;
        let context: ToolModuleInvocationContext =
            serde_json::from_str(&context_json).map_err(error)?;
        if host.is_cancelled()? {
            return Err(ProcessModuleError::new("search was canceled"));
        }
        let query = call
            .args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| ProcessModuleError::new("search requires string arg 'query'"))?;
        let max_results = call
            .args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(20) as usize;
        let mut query = SearchQuery::new(query, context.cwd, max_results).with_path_filters(
            string_array_arg(&call.args, "starts_with")?,
            string_array_arg(&call.args, "ends_with")?,
        );
        if let Some(use_case) = call.args.get("use_case").and_then(Value::as_str) {
            query = query.with_use_case(use_case);
        }
        let chunks = super::run_rg(query).map_err(ProcessModuleError::new)?;
        let result = ToolResult::new(
            call.id,
            true,
            format_search_output(&chunks),
            Vec::new(),
            None,
            json!({"results": chunks.len(), "chunks": chunks}),
        );
        serde_json::to_string(&result).map_err(error)
    }
}

fn format_search_output(chunks: &[ContextChunk]) -> String {
    if chunks.is_empty() {
        return "(no matches)".to_owned();
    }
    chunks
        .iter()
        .map(|chunk| {
            let path = chunk
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| chunk.source.clone());
            let content = chunk.content.trim();
            if let Some(line) = chunk.metadata.get("line").and_then(Value::as_u64) {
                format!("{path}:{line}: {content}")
            } else {
                format!("{path}: {content}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn string_array_arg(args: &Value, name: &str) -> ProcessModuleResult<Vec<String>> {
    let Some(value) = args.get(name) else {
        return Ok(Vec::new());
    };
    let message =
        || ProcessModuleError::new(format!("search arg '{name}' must be an array of strings"));
    value
        .as_array()
        .ok_or_else(message)?
        .iter()
        .map(|item| item.as_str().map(str::to_owned).ok_or_else(message))
        .collect()
}

fn error(error: serde_json::Error) -> ProcessModuleError {
    ProcessModuleError::new(error.to_string())
}
