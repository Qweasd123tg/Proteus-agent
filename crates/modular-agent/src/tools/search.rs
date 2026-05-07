use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{SearchBackend, SearchQuery, Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
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
            "Search the current workspace",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "max_results": { "type": "integer" },
                    "use_case": { "type": "string" },
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
        .with_timeout(20_000)
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
        let starts_with = string_array_arg(&call.args, "starts_with")?;
        let ends_with = string_array_arg(&call.args, "ends_with")?;
        let mut search_query =
            SearchQuery::new(query, ctx.cwd, max_results).with_path_filters(starts_with, ends_with);
        if let Some(use_case) = use_case {
            search_query = search_query.with_use_case(use_case);
        }
        let chunks = self.search.search(search_query).await?;
        Ok(ToolResult::new(
            call.id.clone(),
            true,
            serde_json::to_string_pretty(&chunks)?,
            Vec::new(),
            None,
            json!({ "results": chunks.len() }),
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptySearch;

    #[async_trait]
    impl SearchBackend for EmptySearch {
        async fn search(&self, _query: SearchQuery) -> Result<Vec<crate::domain::ContextChunk>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn search_tool_timeout_exceeds_rg_backend_timeout() {
        let tool = SearchTool::new(Arc::new(EmptySearch));

        assert_eq!(tool.spec().timeout_ms, Some(20_000));
    }
}
