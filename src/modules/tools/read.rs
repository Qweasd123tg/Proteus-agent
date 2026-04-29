use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

#[derive(Debug)]
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".to_owned(),
            description: "Read a UTF-8 file inside the current workspace".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }),
            safety: ToolSafety::ReadOnly,
            timeout_ms: Some(5_000),
            metadata: serde_json::Value::Null,
        }
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let path = required_path(call)?;
        let path = workspace_path(&ctx.cwd, &path).await?;
        let metadata = tokio::fs::metadata(&path)
            .await
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if metadata.is_dir() {
            bail!(
                "path is a directory; use list_dir to list entries: {}",
                path.display()
            );
        }
        let output = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: true,
            output,
            content: Vec::new(),
            error: None,
            metadata: json!({ "path": path }),
        })
    }
}

pub(crate) fn required_path(call: &ToolCall) -> Result<PathBuf> {
    call.args
        .get("path")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("tool '{}' requires string arg 'path'", call.name))
}

pub(crate) async fn workspace_path(cwd: &Path, path: &Path) -> Result<PathBuf> {
    let base = tokio::fs::canonicalize(cwd)
        .await
        .with_context(|| format!("failed to canonicalize cwd {}", cwd.display()))?;
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let canonical = tokio::fs::canonicalize(&target)
        .await
        .with_context(|| format!("failed to canonicalize {}", target.display()))?;
    if !canonical.starts_with(&base) {
        bail!("path escapes workspace: {}", path.display());
    }
    Ok(canonical)
}
