use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
    modules::required_path,
};

#[derive(Debug)]
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".to_owned(),
            description: "Write a UTF-8 file inside the current workspace".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            safety: ToolSafety::WritesFiles,
            timeout_ms: Some(5_000),
            metadata: serde_json::Value::Null,
        }
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let path = required_path(call)?;
        if path.is_absolute() {
            bail!("absolute writes are not allowed: {}", path.display());
        }
        let content = call
            .args
            .get("content")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("write_file requires string arg 'content'"))?;
        let target = ctx.cwd.join(&path);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&target, content)
            .await
            .with_context(|| format!("failed to write {}", target.display()))?;
        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: true,
            output: format!("wrote {} bytes to {}", content.len(), path.display()),
            error: None,
            metadata: json!({ "path": path }),
        })
    }
}
