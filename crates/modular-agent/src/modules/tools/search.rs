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
        ToolSpec {
            name: "search".to_owned(),
            description: "Search the current workspace".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "max_results": { "type": "integer" }
                },
                "required": ["query"]
            }),
            safety: ToolSafety::ReadOnly,
            timeout_ms: Some(10_000),
            metadata: serde_json::Value::Null,
        }
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
        let chunks = self
            .search
            .search(SearchQuery::new(query, ctx.cwd, max_results))
            .await?;
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
