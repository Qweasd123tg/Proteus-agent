use std::process::Stdio;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
};

use std::sync::Arc;

const MCP_STDIO_RESPONSE_LIMIT_BYTES: usize = DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES;

use crate::{
    contracts::{PatchApplier, SearchBackend, Tool, ToolContext, ToolRegistry, ToolSource},
    core::{ConfiguredToolConfig, ConfiguredToolExecutorConfig},
    core::process_output::{
        DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES, annotate_bounded_output, wait_with_bounded_output,
    },
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

use super::{ApplyPatchTool, SearchTool};

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

        let output = wait_with_bounded_output(
            child,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
        )
        .await?;

        let error = if output.status.success() {
            None
        } else if output.stderr.text.is_empty() {
            Some(format!(
                "process tool '{}' exited with status {:?}",
                call.name,
                output.status.code()
            ))
        } else {
            Some(output.stderr.text.clone())
        };
        let metadata = annotate_bounded_output(
            json!({
                "tool": call.name,
                "executor": "process",
                "status": output.status.code(),
            }),
            &output,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
            DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
        );
        Ok(ToolResult::new(
            call.id.clone(),
            output.status.success(),
            output.stdout.text.clone(),
            Vec::new(),
            error,
            metadata,
        ))
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

        let content_text = render_mcp_content(result.get("content"));
        let error = is_error.then(|| content_text.clone());
        let metadata = json!({
            "tool": call.name,
            "executor": "mcp",
            "remote_tool": self.remote_tool,
            "structured_content": result.get("structuredContent").cloned().unwrap_or(Value::Null),
        });
        Ok(ToolResult::new(
            call.id.clone(),
            !is_error,
            content_text,
            Vec::new(),
            error,
            metadata,
        ))
    }
}

pub fn register_configured_tools(
    registry: &mut ToolRegistry,
    configured_tools: &[ConfiguredToolConfig],
    search: Arc<dyn SearchBackend>,
    patch: Arc<dyn PatchApplier>,
) -> Result<()> {
    for configured in configured_tools {
        let source = configured_tool_source(configured);
        let spec = configured_tool_spec(configured);
        match &configured.executor {
            ConfiguredToolExecutorConfig::Native { handler } => {
                let inner = configured_native_handler(handler, search.clone(), patch.clone())?;
                registry.register_with_source(source, ConfiguredNativeTool::new(spec, inner))?;
            }
            ConfiguredToolExecutorConfig::Process { command, args } => {
                registry.register_with_source(
                    source,
                    ConfiguredProcessTool::new(spec, command.clone(), args.clone()),
                )?;
            }
            ConfiguredToolExecutorConfig::Mcp {
                server: _,
                command,
                args,
                tool,
                protocol_version,
            } => registry.register_with_source(
                source,
                ConfiguredMcpTool::new(
                    spec,
                    command.clone(),
                    args.clone(),
                    tool.clone(),
                    protocol_version.clone(),
                ),
            )?,
        }
    }
    Ok(())
}

fn configured_tool_source(configured: &ConfiguredToolConfig) -> ToolSource {
    match &configured.executor {
        ConfiguredToolExecutorConfig::Native { .. } => ToolSource::Config {
            origin: "config:native".to_owned(),
        },
        ConfiguredToolExecutorConfig::Mcp {
            server, command, ..
        } => ToolSource::Mcp {
            server: server.clone().unwrap_or_else(|| command.clone()),
        },
        ConfiguredToolExecutorConfig::Process { .. } => ToolSource::Config {
            origin: "config".to_owned(),
        },
    }
}

fn configured_tool_spec(configured: &ConfiguredToolConfig) -> ToolSpec {
    let spec = ToolSpec::new(
        configured.name.clone(),
        configured.description.clone(),
        configured.input_schema.clone(),
        effective_configured_tool_safety(configured),
    )
    .with_metadata(configured.metadata.clone());
    if let Some(timeout_ms) = configured.timeout_ms {
        spec.with_timeout(timeout_ms)
    } else {
        spec
    }
}

fn effective_configured_tool_safety(configured: &ConfiguredToolConfig) -> ToolSafety {
    match &configured.executor {
        ConfiguredToolExecutorConfig::Native { handler } => {
            max_tool_safety(configured.safety.clone(), native_handler_safety(handler))
        }
        ConfiguredToolExecutorConfig::Mcp { .. } => match configured.safety {
            ToolSafety::Dangerous => ToolSafety::Dangerous,
            ToolSafety::Network => ToolSafety::Network,
            ToolSafety::ReadOnly | ToolSafety::WritesFiles | ToolSafety::RunsCommands => {
                ToolSafety::RunsCommands
            }
            _ => ToolSafety::Dangerous,
        },
        ConfiguredToolExecutorConfig::Process { .. } => match configured.safety {
            ToolSafety::Dangerous => ToolSafety::Dangerous,
            ToolSafety::Network => ToolSafety::Network,
            ToolSafety::ReadOnly | ToolSafety::WritesFiles | ToolSafety::RunsCommands => {
                ToolSafety::RunsCommands
            }
            _ => ToolSafety::Dangerous,
        },
    }
}

fn configured_native_handler(
    handler: &str,
    search: Arc<dyn SearchBackend>,
    patch: Arc<dyn PatchApplier>,
) -> Result<Arc<dyn Tool>> {
    match handler {
        "apply_patch" => Ok(Arc::new(ApplyPatchTool::new(patch))),
        "search" => Ok(Arc::new(SearchTool::new(search))),
        other => bail!(
            "unsupported native tool handler: '{other}'. File I/O (read_file, \
             write_file, list_dir) and shell are now provided by the `file-tools` \
             and `shell-tool` plugins — use tools.enabled with the plugin names, \
             not configured.native.handler."
        ),
    }
}

fn native_handler_safety(handler: &str) -> ToolSafety {
    match handler {
        "search" => ToolSafety::ReadOnly,
        "apply_patch" => ToolSafety::WritesFiles,
        _ => ToolSafety::Dangerous,
    }
}

fn max_tool_safety(left: ToolSafety, right: ToolSafety) -> ToolSafety {
    if tool_safety_rank(&left) >= tool_safety_rank(&right) {
        left
    } else {
        right
    }
}

fn tool_safety_rank(safety: &ToolSafety) -> u8 {
    match safety {
        ToolSafety::ReadOnly => 0,
        ToolSafety::WritesFiles => 1,
        ToolSafety::RunsCommands => 2,
        ToolSafety::Network => 3,
        ToolSafety::Dangerous => 4,
        _ => 5,
    }
}

async fn write_json_line(stdin: &mut ChildStdin, message: Value) -> Result<()> {
    stdin.write_all(message.to_string().as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_json_line<R>(stdout: &mut R) -> Result<Value>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::with_capacity(MCP_STDIO_RESPONSE_LIMIT_BYTES.min(8192));

    loop {
        let buffer = stdout.fill_buf().await?;
        if buffer.is_empty() {
            if line.is_empty() {
                bail!("MCP server closed stdout before sending a response");
            }
            break;
        }

        let bytes_to_take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |position| position + 1);
        if line.len().saturating_add(bytes_to_take) > MCP_STDIO_RESPONSE_LIMIT_BYTES {
            bail!("MCP response exceeded {MCP_STDIO_RESPONSE_LIMIT_BYTES} bytes before newline");
        }

        line.extend_from_slice(&buffer[..bytes_to_take]);
        stdout.consume(bytes_to_take);

        if line.last() == Some(&b'\n') {
            break;
        }
    }

    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }

    let line = std::str::from_utf8(&line)?;
    serde_json::from_str(line).map_err(Into::into)
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

#[cfg(test)]
mod tests {
    use crate::{
        contracts::{Tool, ToolContext},
        domain::{ToolCall, ToolSafety, ToolSpec, new_call_id},
    };

    use super::*;

    #[tokio::test]
    async fn configured_process_output_is_bounded_before_returning_result() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let tool = ConfiguredProcessTool::new(
            ToolSpec::new(
                "big_process",
                "prints a large output",
                json!({ "type": "object" }),
                ToolSafety::RunsCommands,
            )
            .with_timeout(30_000),
            "sh".to_owned(),
            vec![
                "-c".to_owned(),
                "i=0; while [ \"$i\" -lt 5000 ]; do printf 0123456789; i=$((i+1)); done".to_owned(),
            ],
        );
        let call = ToolCall::new(new_call_id(), "big_process".to_owned(), json!({}));

        let result = tool
            .invoke(&call, ToolContext::new(cwd.path().to_path_buf()))
            .await
            .expect("process result");

        assert!(result.ok);
        assert_eq!(result.output.len(), DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES);
        assert_eq!(result.metadata["stdout_truncated"], true);
        assert_eq!(result.metadata["stdout_original_bytes"], 50_000);
    }

    #[tokio::test]
    async fn mcp_json_line_rejects_oversized_response_without_newline() {
        let response = vec![b' '; MCP_STDIO_RESPONSE_LIMIT_BYTES + 1];
        let mut stdout = BufReader::new(&response[..]);

        let error = read_json_line(&mut stdout)
            .await
            .expect_err("oversized MCP response should fail");

        assert!(
            error
                .to_string()
                .contains("MCP response exceeded 20000 bytes before newline")
        );
    }
}
