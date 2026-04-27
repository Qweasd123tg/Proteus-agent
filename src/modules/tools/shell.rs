use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::json;
use tokio::process::Command;

use crate::{
    contracts::{Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
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
            .output()
            .await?;

        let mut rendered = String::new();
        rendered.push_str(&String::from_utf8_lossy(&output.stdout));
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            if !rendered.is_empty() {
                rendered.push('\n');
            }
            rendered.push_str(&stderr);
        }

        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: output.status.success(),
            output: rendered,
            error: output.status.code().and_then(|code| {
                if output.status.success() {
                    None
                } else {
                    Some(format!("process exited with code {code}"))
                }
            }),
            metadata: json!({ "status": output.status.code() }),
        })
    }
}
