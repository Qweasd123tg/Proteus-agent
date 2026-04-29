use std::process::Stdio;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::json;
use tokio::process::Command;

use crate::{
    contracts::{Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
    modules::process_output::{
        DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES, annotate_bounded_output, wait_with_bounded_output,
    },
};

#[derive(Debug)]
pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shell".to_owned(),
            description: "Run a shell command in the current workspace".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
            safety: ToolSafety::RunsCommands,
            timeout_ms: Some(30_000),
            metadata: serde_json::Value::Null,
        }
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let command = call
            .args
            .get("command")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("shell requires string arg 'command'"))?;
        let output = Command::new("sh")
            .arg("-lc")
            .arg(command)
            .current_dir(ctx.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let output = wait_with_bounded_output(
            output,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
        )
        .await?;

        let mut rendered = String::new();
        rendered.push_str(&output.stdout.text);
        if !output.stderr.text.is_empty() {
            if !rendered.is_empty() {
                rendered.push('\n');
            }
            rendered.push_str(&output.stderr.text);
        }

        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: output.status.success(),
            output: rendered,
            content: Vec::new(),
            error: output.status.code().and_then(|code| {
                if output.status.success() {
                    None
                } else {
                    Some(format!("process exited with code {code}"))
                }
            }),
            metadata: annotate_bounded_output(
                json!({ "status": output.status.code() }),
                &output,
                DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
                DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        contracts::{Tool, ToolContext},
        domain::{ToolCall, new_call_id},
    };

    use super::*;

    #[tokio::test]
    async fn shell_output_is_bounded_before_returning_result() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let call = ToolCall {
            id: new_call_id(),
            name: "shell".to_owned(),
            args: json!({
                "command": "i=0; while [ \"$i\" -lt 5000 ]; do printf 0123456789; i=$((i+1)); done"
            }),
        };

        let result = ShellTool
            .invoke(&call, ToolContext::new(cwd.path().to_path_buf()))
            .await
            .expect("shell result");

        assert!(result.ok);
        assert_eq!(result.output.len(), DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES);
        assert_eq!(result.metadata["stdout_truncated"], true);
        assert_eq!(result.metadata["stdout_original_bytes"], 50_000);
    }
}
