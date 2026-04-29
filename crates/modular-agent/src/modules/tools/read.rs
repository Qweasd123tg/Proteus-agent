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
            description: "Read a UTF-8 file inside the current workspace, optionally by line range"
                .to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "start_line": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "1-based first line to read. Defaults to 1."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of lines to return."
                    },
                    "line_numbers": {
                        "type": "boolean",
                        "description": "Prefix each returned line with its 1-based line number."
                    }
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
        let content = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        let options = ReadOptions::from_call(call)?;
        let (output, metadata) = render_read_output(&content, &path, options);
        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: true,
            output,
            content: Vec::new(),
            error: None,
            metadata,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ReadOptions {
    start_line: Option<usize>,
    limit: Option<usize>,
    line_numbers: bool,
}

impl ReadOptions {
    fn from_call(call: &ToolCall) -> Result<Self> {
        Ok(Self {
            start_line: optional_positive_usize(call, "start_line")?,
            limit: optional_positive_usize(call, "limit")?,
            line_numbers: call
                .args
                .get("line_numbers")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
        })
    }

    fn is_default(self) -> bool {
        self.start_line.is_none() && self.limit.is_none() && !self.line_numbers
    }
}

fn optional_positive_usize(call: &ToolCall, key: &str) -> Result<Option<usize>> {
    let Some(value) = call.args.get(key) else {
        return Ok(None);
    };
    let Some(number) = value.as_u64() else {
        bail!("tool '{}' requires integer arg '{key}'", call.name);
    };
    if number == 0 {
        bail!("tool '{}' requires positive integer arg '{key}'", call.name);
    }
    Ok(Some(number as usize))
}

fn render_read_output(
    content: &str,
    path: &Path,
    options: ReadOptions,
) -> (String, serde_json::Value) {
    let total_lines = content.lines().count();
    if options.is_default() {
        return (
            content.to_owned(),
            json!({
                "path": path,
                "total_lines": total_lines,
                "returned_lines": total_lines,
                "truncated": false,
            }),
        );
    }

    let start_line = options.start_line.unwrap_or(1);
    let limit = options.limit.unwrap_or(usize::MAX);
    let start_index = start_line.saturating_sub(1);
    let mut returned = 0usize;
    let mut rendered = Vec::new();
    for (index, line) in content.lines().enumerate().skip(start_index).take(limit) {
        returned += 1;
        if options.line_numbers {
            rendered.push(format!("{}\t{}", index + 1, line));
        } else {
            rendered.push(line.to_owned());
        }
    }
    let end_line = if returned == 0 {
        None
    } else {
        Some(start_line + returned - 1)
    };
    let truncated = start_index + returned < total_lines;

    (
        rendered.join("\n"),
        json!({
            "path": path,
            "start_line": start_line,
            "end_line": end_line,
            "limit": options.limit,
            "line_numbers": options.line_numbers,
            "total_lines": total_lines,
            "returned_lines": returned,
            "truncated": truncated,
        }),
    )
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::{ToolCall, new_call_id};

    #[tokio::test]
    async fn read_file_without_range_preserves_full_content() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("sample.txt"), "alpha\nbeta\n").expect("sample");
        let tool = ReadFileTool;

        let result = tool
            .invoke(
                &ToolCall {
                    id: new_call_id(),
                    name: "read_file".to_owned(),
                    args: json!({ "path": "sample.txt" }),
                },
                ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .expect("read file");

        assert_eq!(result.output, "alpha\nbeta\n");
        assert_eq!(result.metadata["total_lines"], 2);
        assert_eq!(result.metadata["truncated"], false);
    }

    #[tokio::test]
    async fn read_file_can_return_numbered_line_range() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("sample.txt"), "alpha\nbeta\ngamma\ndelta\n")
            .expect("sample");
        let tool = ReadFileTool;

        let result = tool
            .invoke(
                &ToolCall {
                    id: new_call_id(),
                    name: "read_file".to_owned(),
                    args: json!({
                        "path": "sample.txt",
                        "start_line": 2,
                        "limit": 2,
                        "line_numbers": true,
                    }),
                },
                ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .expect("read file");

        assert_eq!(result.output, "2\tbeta\n3\tgamma");
        assert_eq!(result.metadata["start_line"], 2);
        assert_eq!(result.metadata["end_line"], 3);
        assert_eq!(result.metadata["returned_lines"], 2);
        assert_eq!(result.metadata["truncated"], true);
    }

    #[tokio::test]
    async fn read_file_rejects_zero_line_limit() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("sample.txt"), "alpha\n").expect("sample");
        let tool = ReadFileTool;

        let error = tool
            .invoke(
                &ToolCall {
                    id: new_call_id(),
                    name: "read_file".to_owned(),
                    args: json!({ "path": "sample.txt", "limit": 0 }),
                },
                ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("requires positive integer arg 'limit'")
        );
    }
}
