use std::path::PathBuf;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

use super::read::workspace_path;

#[derive(Debug)]
pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "list_dir",
            "List files and directories inside the current workspace",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path relative to the workspace. Defaults to '.'."
                    }
                }
            }),
            ToolSafety::ReadOnly,
        )
        .with_timeout(5_000)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let requested_path = optional_path(call);
        let path = workspace_path(&ctx.cwd, &requested_path).await?;
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .with_context(|| format!("failed to list {}", path.display()))?;
        let mut rendered = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let file_type = entry.file_type().await?;
            let kind = if file_type.is_dir() {
                "dir"
            } else if file_type.is_file() {
                "file"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "other"
            };
            rendered.push(format!("{kind}\t{file_name}"));
        }

        rendered.sort();
        let entry_count = rendered.len();
        Ok(ToolResult::ok(call.id.clone(), rendered.join("\n")).with_metadata(json!({
            "path": path,
            "entries": entry_count,
        })))
    }
}

fn optional_path(call: &ToolCall) -> PathBuf {
    call.args
        .get("path")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
