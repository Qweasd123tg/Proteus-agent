use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{SearchBackend, SearchQuery, Tool, ToolContext},
    domain::{ContextChunk, ToolCall, ToolResult, ToolSafety, ToolSpec},
};

pub struct SearchTool {
    search: Arc<dyn SearchBackend>,
}

impl SearchTool {
    pub fn new(search: Arc<dyn SearchBackend>) -> Self {
        Self { search }
    }
}

#[async_trait]
impl Tool for SearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "search",
            "Search the current workspace through the configured SearchBackend. In the standard coding profile this is backed by ripgrep; use grep for raw regex line search and search for backend/context-aware workspace search.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Text or regex-like query to search for."
                    },
                    "max_results": { "type": "integer" },
                    "use_case": { "type": "string" },
                    "path": {
                        "type": "string",
                        "description": "Optional workspace-relative path prefix to search within."
                    },
                    "starts_with": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "ends_with": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["query"]
            }),
            ToolSafety::ReadOnly,
        )
        .with_timeout(60_000)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let query = call
            .args
            .get("query")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("search requires string arg 'query'"))?;
        let max_results = call
            .args
            .get("max_results")
            .and_then(|value| value.as_u64())
            .unwrap_or(20) as usize;
        let use_case = call.args.get("use_case").and_then(|value| value.as_str());
        let mut starts_with = string_array_arg(&call.args, "starts_with")?;
        if let Some(path) = call.args.get("path").and_then(|value| value.as_str()) {
            starts_with.push(normalize_path_prefix(path));
        }
        let ends_with = string_array_arg(&call.args, "ends_with")?;
        let mut search_query =
            SearchQuery::new(query, ctx.cwd, max_results).with_path_filters(starts_with, ends_with);
        if let Some(use_case) = use_case {
            search_query = search_query.with_use_case(use_case);
        }
        let chunks = self.search.search(search_query).await?;
        let output = format_search_output(&chunks);
        let raw_chunks = serde_json::to_value(&chunks)?;
        let results = chunks.len();
        Ok(ToolResult::new(
            call.id.clone(),
            true,
            output,
            Vec::new(),
            None,
            json!({
                "results": results,
                "chunks": raw_chunks,
            }),
        ))
    }
}

fn format_search_output(chunks: &[ContextChunk]) -> String {
    if chunks.is_empty() {
        return "(no matches)".to_owned();
    }

    chunks
        .iter()
        .map(format_search_chunk)
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_search_chunk(chunk: &ContextChunk) -> String {
    let path = chunk
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| chunk.source.clone());
    let content = chunk.content.trim();
    if let Some(line) = chunk.metadata.get("line").and_then(|line| line.as_u64()) {
        format!("{path}:{line}: {content}")
    } else {
        format!("{path}: {content}")
    }
}

fn string_array_arg(args: &serde_json::Value, name: &str) -> Result<Vec<String>> {
    let Some(value) = args.get(name) else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(anyhow!("search arg '{name}' must be an array of strings"));
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("search arg '{name}' must be an array of strings"))
        })
        .collect()
}

fn normalize_path_prefix(path: &str) -> String {
    let trimmed = path.trim().trim_start_matches("./");
    if trimmed.is_empty() || trimmed == "." {
        String::new()
    } else if trimmed.ends_with('/') {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingSearch {
        queries: Mutex<Vec<SearchQuery>>,
        chunks: Vec<ContextChunk>,
    }

    #[async_trait]
    impl SearchBackend for RecordingSearch {
        async fn search(&self, query: SearchQuery) -> Result<Vec<crate::domain::ContextChunk>> {
            self.queries.lock().unwrap().push(query);
            Ok(self.chunks.clone())
        }
    }

    #[test]
    fn search_tool_timeout_exceeds_rg_backend_timeout() {
        let tool = SearchTool::new(Arc::new(RecordingSearch {
            queries: Mutex::new(Vec::new()),
            chunks: Vec::new(),
        }));

        assert_eq!(tool.spec().timeout_ms, Some(60_000));
    }

    #[tokio::test]
    async fn search_tool_maps_path_alias_to_starts_with_filter() {
        let search = Arc::new(RecordingSearch {
            queries: Mutex::new(Vec::new()),
            chunks: Vec::new(),
        });
        let tool = SearchTool::new(search.clone());
        let call = ToolCall::new(
            crate::domain::new_call_id(),
            "search",
            json!({
                "query": "needle",
                "path": "src",
                "starts_with": ["tests/"],
                "ends_with": [".rs"]
            }),
        );

        let result = tool
            .invoke(&call, ToolContext::new(".".into()))
            .await
            .unwrap();

        assert!(result.ok);
        let queries = search.queries.lock().unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].starts_with, ["tests/", "src/"]);
        assert_eq!(queries[0].ends_with, [".rs"]);
    }

    #[tokio::test]
    async fn search_tool_outputs_human_readable_matches_and_keeps_raw_chunks() {
        let tool = SearchTool::new(Arc::new(RecordingSearch {
            queries: Mutex::new(Vec::new()),
            chunks: vec![
                ContextChunk::new("rg", "let needle = true;")
                    .with_path("src/main.rs".into())
                    .with_metadata(json!({ "line": 42 })),
                ContextChunk::new("memory", "needle remembered"),
            ],
        }));
        let call = ToolCall::new(
            crate::domain::new_call_id(),
            "search",
            json!({ "query": "needle" }),
        );

        let result = tool
            .invoke(&call, ToolContext::new(".".into()))
            .await
            .unwrap();

        assert_eq!(
            result.output,
            "src/main.rs:42: let needle = true;\nmemory: needle remembered"
        );
        assert_eq!(result.metadata["results"], 2);
        assert_eq!(result.metadata["chunks"][0]["path"], "src/main.rs");
    }

    #[tokio::test]
    async fn search_tool_outputs_no_matches_instead_of_empty_json_array() {
        let tool = SearchTool::new(Arc::new(RecordingSearch {
            queries: Mutex::new(Vec::new()),
            chunks: Vec::new(),
        }));
        let call = ToolCall::new(
            crate::domain::new_call_id(),
            "search",
            json!({ "query": "absent" }),
        );

        let result = tool
            .invoke(&call, ToolContext::new(".".into()))
            .await
            .unwrap();

        assert_eq!(result.output, "(no matches)");
        assert_eq!(result.metadata["results"], 0);
        assert!(result.metadata["chunks"].as_array().unwrap().is_empty());
    }
}
