use std::process::Stdio;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
};

use std::sync::Arc;

use crate::{
    contracts::{Tool, ToolContext},
    domain::{ToolCall, ToolResult, ToolSpec},
};

#[derive(Clone)]
pub struct ConfiguredNativeTool {
    spec: ToolSpec,
    inner: Arc<dyn Tool>,
}

#[derive(Debug, Clone)]
pub struct ConfiguredProcessTool {
    spec: ToolSpec,
    command: String,
    args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ConfiguredMcpTool {
    spec: ToolSpec,
    command: String,
    args: Vec<String>,
    remote_tool: String,
    protocol_version: String,
}

impl ConfiguredNativeTool {
    pub fn new(spec: ToolSpec, inner: Arc<dyn Tool>) -> Self {
        Self { spec, inner }
    }
}

impl ConfiguredProcessTool {
    pub fn new(spec: ToolSpec, command: String, args: Vec<String>) -> Self {
        Self {
            spec,
            command,
            args,
        }
    }
}

impl ConfiguredMcpTool {
    pub fn new(
        spec: ToolSpec,
        command: String,
        args: Vec<String>,
        remote_tool: String,
        protocol_version: String,
    ) -> Self {
        Self {
            spec,
            command,
            args,
            remote_tool,
            protocol_version,
        }
    }
}

#[async_trait]
impl Tool for ConfiguredNativeTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        self.inner.invoke(call, ctx).await
    }
}

#[async_trait]
impl Tool for ConfiguredProcessTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .current_dir(ctx.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open process tool stdin"))?;
        stdin.write_all(call.args.to_string().as_bytes()).await?;
        drop(stdin);

        let output = child.wait_with_output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: output.status.success(),
            output: stdout.to_string(),
            error: if output.status.success() {
                None
            } else if stderr.is_empty() {
                Some(format!(
                    "process tool '{}' exited with status {:?}",
                    call.name,
                    output.status.code()
                ))
            } else {
                Some(stderr.to_string())
            },
            metadata: json!({
                "tool": call.name,
                "executor": "process",
                "status": output.status.code(),
            }),
        })
    }
}

#[async_trait]
impl Tool for ConfiguredMcpTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .current_dir(ctx.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open MCP server stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to open MCP server stdout"))?;
        let mut stdout = BufReader::new(stdout);

        let result = self.call_mcp(call, &mut stdin, &mut stdout).await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        result
    }
}

impl ConfiguredMcpTool {
    async fn call_mcp(
        &self,
        call: &ToolCall,
        stdin: &mut ChildStdin,
        stdout: &mut BufReader<ChildStdout>,
    ) -> Result<ToolResult> {
        write_json_line(
            stdin,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": self.protocol_version,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "modular-agent",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
        )
        .await?;
        let initialize = read_json_line(stdout).await?;
        ensure_jsonrpc_success(&initialize, 1)?;

        write_json_line(
            stdin,
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
        )
        .await?;

        write_json_line(
            stdin,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": self.remote_tool,
                    "arguments": call.args
                }
            }),
        )
        .await?;
        let response = read_json_line(stdout).await?;
        let result = ensure_jsonrpc_success(&response, 2)?;
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: !is_error,
            output: render_mcp_content(result.get("content")),
            error: is_error.then(|| render_mcp_content(result.get("content"))),
            metadata: json!({
                "tool": call.name,
                "executor": "mcp",
                "remote_tool": self.remote_tool,
                "structured_content": result.get("structuredContent").cloned().unwrap_or(Value::Null),
            }),
        })
    }
}

async fn write_json_line(stdin: &mut ChildStdin, message: Value) -> Result<()> {
    stdin.write_all(message.to_string().as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_json_line(stdout: &mut BufReader<ChildStdout>) -> Result<Value> {
    let mut line = String::new();
    let bytes = stdout.read_line(&mut line).await?;
    if bytes == 0 {
        bail!("MCP server closed stdout before sending a response");
    }
    serde_json::from_str(line.trim_end()).map_err(Into::into)
}

fn ensure_jsonrpc_success(response: &Value, expected_id: i64) -> Result<&Value> {
    let id = response
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("MCP response missing numeric id"))?;
    if id != expected_id {
        bail!("MCP response id {id} did not match expected id {expected_id}");
    }
    if let Some(error) = response.get("error") {
        bail!("MCP error response: {error}");
    }
    response
        .get("result")
        .ok_or_else(|| anyhow!("MCP response missing result"))
}

fn render_mcp_content(content: Option<&Value>) -> String {
    let Some(Value::Array(items)) = content else {
        return String::new();
    };
    items
        .iter()
        .map(|item| match item.get("type").and_then(Value::as_str) {
            Some("text") => item
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            _ => item.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
