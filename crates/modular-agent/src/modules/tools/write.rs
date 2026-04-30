use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

use super::read::required_path;

#[derive(Debug)]
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "write_file",
            "Write a UTF-8 file inside the current workspace",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            ToolSafety::WritesFiles,
        )
        .with_timeout(5_000)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let path = required_path(call)?;
        let target = writable_workspace_path(&ctx.cwd, &path).await?;
        let content = required_content_arg(call, "content")?;
        tokio::fs::write(&target, content)
            .await
            .with_context(|| format!("failed to write {}", target.display()))?;
        Ok(ToolResult::ok(
            call.id.clone(),
            format!("wrote {} bytes to {}", content.len(), path.display()),
        )
        .with_metadata(json!({ "path": path })))
    }
}

pub(crate) fn required_content_arg<'a>(call: &'a ToolCall, name: &str) -> Result<&'a str> {
    call.args
        .get(name)
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("{} requires string arg '{}'", call.name, name))
}

pub(crate) async fn writable_workspace_path(cwd: &Path, path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        bail!("absolute writes are not allowed: {}", path.display());
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => bail!("path escapes workspace: {}", path.display()),
            Component::RootDir | Component::Prefix(_) => {
                bail!("absolute writes are not allowed: {}", path.display())
            }
        }
    }

    if clean.as_os_str().is_empty() {
        bail!("write path must not be empty");
    }

    let base = tokio::fs::canonicalize(cwd)
        .await
        .with_context(|| format!("failed to canonicalize cwd {}", cwd.display()))?;
    let target = base.join(&clean);

    if let Ok(canonical_target) = tokio::fs::canonicalize(&target).await {
        if !canonical_target.starts_with(&base) {
            bail!("path escapes workspace: {}", path.display());
        }
        return Ok(canonical_target);
    }

    let parent = target
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("write path has no parent: {}", path.display()))?;
    tokio::fs::create_dir_all(&parent).await?;
    let canonical_parent = tokio::fs::canonicalize(&parent)
        .await
        .with_context(|| format!("failed to canonicalize {}", parent.display()))?;
    if !canonical_parent.starts_with(&base) {
        bail!("path escapes workspace: {}", path.display());
    }

    let file_name = target
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("write path must name a file: {}", path.display()))?;
    Ok(canonical_parent.join(file_name))
}
