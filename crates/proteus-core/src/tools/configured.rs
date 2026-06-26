use std::{
    io::{BufRead, BufReader as StdBufReader, Write},
    path::{Path, PathBuf},
    process::{Child as StdChild, ChildStdin as StdChildStdin, Command as StdCommand, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
#[cfg(test)]
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use tokio::{io::AsyncWriteExt, process::Command};

const MCP_STDIO_RESPONSE_LIMIT_BYTES: usize = DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES;

use crate::{
    contracts::{PatchApplier, SearchBackend, Tool, ToolContext, ToolRegistry, ToolSource},
    core::process_output::{
        DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES, annotate_bounded_output, wait_with_bounded_output,
    },
    core::{ConfiguredMcpServerConfig, ConfiguredToolConfig, ConfiguredToolExecutorConfig},
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

#[derive(Clone)]
pub struct ConfiguredMcpTool {
    spec: ToolSpec,
    remote_tool: String,
    host: Arc<McpStdioHost>,
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
    fn new(spec: ToolSpec, remote_tool: String, host: Arc<McpStdioHost>) -> Self {
        Self {
            spec,
            remote_tool,
            host,
        }
    }
}

#[derive(Debug)]
struct McpStdioHost {
    server_name: String,
    command: String,
    args: Vec<String>,
    protocol_version: String,
    cwd: PathBuf,
    timeout: Duration,
    session: Mutex<Option<McpStdioSession>>,
}

impl McpStdioHost {
    fn new(
        server_name: String,
        command: String,
        args: Vec<String>,
        protocol_version: String,
        cwd: PathBuf,
        timeout: Duration,
    ) -> Self {
        Self {
            server_name,
            command,
            args,
            protocol_version,
            cwd,
            timeout,
            session: Mutex::new(None),
        }
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    fn call_tool(&self, remote_tool: &str, args: Value, timeout: Duration) -> Result<Value> {
        let mut session = self.lock_session()?;
        self.ensure_session(&mut session)?;
        let result = session
            .as_mut()
            .expect("MCP session initialized")
            .call_tool(remote_tool, args, timeout);
        if result.is_err() {
            *session = None;
        }
        result
    }

    fn list_tools(&self, server: &ConfiguredMcpServerConfig) -> Result<Vec<DiscoveredMcpTool>> {
        let mut session = self.lock_session()?;
        self.ensure_session(&mut session)?;
        let result = session
            .as_mut()
            .expect("MCP session initialized")
            .list_tools(server, self.timeout);
        if result.is_err() {
            *session = None;
        }
        result
    }

    fn lock_session(&self) -> Result<MutexGuard<'_, Option<McpStdioSession>>> {
        self.session
            .lock()
            .map_err(|_| anyhow!("MCP host '{}' session lock poisoned", self.server_name))
    }

    fn ensure_session(&self, session: &mut Option<McpStdioSession>) -> Result<()> {
        if session.is_none() {
            *session = Some(McpStdioSession::start(
                &self.server_name,
                &self.command,
                &self.args,
                &self.protocol_version,
                &self.cwd,
                self.timeout,
            )?);
        }
        Ok(())
    }
}

struct McpStdioSession {
    server_name: String,
    child: StdChild,
    stdin: StdChildStdin,
    stdout_rx: Receiver<Result<Value>>,
    next_request_id: i64,
}

impl std::fmt::Debug for McpStdioSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpStdioSession")
            .field("server_name", &self.server_name)
            .field("next_request_id", &self.next_request_id)
            .finish_non_exhaustive()
    }
}

impl McpStdioSession {
    fn start(
        server_name: &str,
        command: &str,
        args: &[String],
        protocol_version: &str,
        cwd: &Path,
        timeout: Duration,
    ) -> Result<Self> {
        let mut child = StdCommand::new(command)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open MCP server stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to open MCP server stdout"))?;
        let stdout_rx = spawn_sync_json_line_reader(stdout);

        let mut session = Self {
            server_name: server_name.to_owned(),
            child,
            stdin,
            stdout_rx,
            next_request_id: 1,
        };
        session.initialize(protocol_version, timeout)?;
        Ok(session)
    }

    fn initialize(&mut self, protocol_version: &str, timeout: Duration) -> Result<()> {
        let request_id = self.next_request_id();
        sync_write_json_line(
            &mut self.stdin,
            json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "initialize",
                "params": {
                    "protocolVersion": protocol_version,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "proteus-core",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
        )?;
        self.recv_success(request_id, timeout)?;

        sync_write_json_line(
            &mut self.stdin,
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
        )?;
        Ok(())
    }

    fn list_tools(
        &mut self,
        server: &ConfiguredMcpServerConfig,
        timeout: Duration,
    ) -> Result<Vec<DiscoveredMcpTool>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let request_id = self.next_request_id();
            let params = cursor
                .as_ref()
                .map(|cursor| json!({ "cursor": cursor }))
                .unwrap_or_else(|| json!({}));
            sync_write_json_line(
                &mut self.stdin,
                json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "tools/list",
                    "params": params
                }),
            )?;
            let result = self.recv_success(request_id, timeout)?;
            tools.extend(mcp_tools_from_list_result(server, &result)?);
            cursor = next_mcp_cursor(&result);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    fn call_tool(&mut self, remote_tool: &str, args: Value, timeout: Duration) -> Result<Value> {
        let request_id = self.next_request_id();
        sync_write_json_line(
            &mut self.stdin,
            json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "tools/call",
                "params": {
                    "name": remote_tool,
                    "arguments": args
                }
            }),
        )?;
        self.recv_success(request_id, timeout)
    }

    fn recv_success(&mut self, expected_id: i64, timeout: Duration) -> Result<Value> {
        recv_sync_jsonrpc_success(&self.stdout_rx, expected_id, timeout, &mut self.child)
    }

    fn next_request_id(&mut self) -> i64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        request_id
    }
}

impl Drop for McpStdioSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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
        if ctx.cancellation.is_cancelled() {
            bail!("tool call canceled");
        }

        let host = Arc::clone(&self.host);
        let remote_tool = self.remote_tool.clone();
        let args = call.args.clone();
        let timeout = self
            .spec
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or_else(|| host.timeout());

        let result =
            tokio::task::spawn_blocking(move || host.call_tool(&remote_tool, args, timeout))
                .await??;
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
    mcp_servers: &[ConfiguredMcpServerConfig],
    cwd: &Path,
    search: Arc<dyn SearchBackend>,
    patch: Arc<dyn PatchApplier>,
) -> Result<()> {
    register_discovered_mcp_tools(registry, mcp_servers, cwd)?;

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
                server,
                command,
                args,
                tool,
                protocol_version,
            } => {
                let host = Arc::new(McpStdioHost::new(
                    server.clone().unwrap_or_else(|| command.clone()),
                    command.clone(),
                    args.clone(),
                    protocol_version.clone(),
                    cwd.to_path_buf(),
                    Duration::from_millis(configured.timeout_ms.unwrap_or(30_000)),
                ));
                registry.register_with_source(
                    source,
                    ConfiguredMcpTool::new(spec, tool.clone(), host),
                )?;
            }
        }
    }
    Ok(())
}

fn register_discovered_mcp_tools(
    registry: &mut ToolRegistry,
    mcp_servers: &[ConfiguredMcpServerConfig],
    cwd: &Path,
) -> Result<()> {
    for server in mcp_servers {
        let host = configured_mcp_server_host(server, cwd);
        let discovered = host.list_tools(server)?;
        for discovered_tool in discovered {
            registry.register_with_source(
                ToolSource::Mcp {
                    server: server.name.clone(),
                },
                ConfiguredMcpTool::new(
                    discovered_tool.spec,
                    discovered_tool.remote_tool,
                    Arc::clone(&host),
                ),
            )?;
        }
    }
    Ok(())
}

fn configured_mcp_server_host(server: &ConfiguredMcpServerConfig, cwd: &Path) -> Arc<McpStdioHost> {
    Arc::new(McpStdioHost::new(
        server.name.clone(),
        server.command.clone(),
        server.args.clone(),
        server.protocol_version.clone(),
        cwd.to_path_buf(),
        Duration::from_millis(server.timeout_ms.unwrap_or(30_000)),
    ))
}

#[derive(Debug)]
struct DiscoveredMcpTool {
    remote_tool: String,
    spec: ToolSpec,
}

#[cfg(test)]
fn discover_mcp_tools(
    server: &ConfiguredMcpServerConfig,
    cwd: &Path,
) -> Result<Vec<DiscoveredMcpTool>> {
    configured_mcp_server_host(server, cwd).list_tools(server)
}

fn spawn_sync_json_line_reader<R>(reader: R) -> Receiver<Result<Value>>
where
    R: std::io::Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = StdBufReader::new(reader);
        loop {
            let value = sync_read_json_line(&mut reader);
            let done = value.is_err();
            if tx.send(value).is_err() || done {
                break;
            }
        }
    });
    rx
}

fn recv_sync_jsonrpc_success(
    rx: &Receiver<Result<Value>>,
    expected_id: i64,
    timeout: Duration,
    child: &mut std::process::Child,
) -> Result<Value> {
    let started = Instant::now();
    loop {
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "MCP server did not send response id {} within {}ms",
                expected_id,
                timeout.as_millis()
            );
        }

        let remaining = timeout - elapsed;
        let response = match rx.recv_timeout(remaining) {
            Ok(value) => value?,
            Err(RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "MCP server did not send response id {} within {}ms",
                    expected_id,
                    timeout.as_millis()
                );
            }
            Err(RecvTimeoutError::Disconnected) => bail!("MCP server stdout reader stopped"),
        };

        let Some(id) = response.get("id") else {
            continue;
        };
        let Some(id) = id.as_i64() else {
            bail!("MCP response id is not numeric: {id}");
        };
        if id != expected_id {
            bail!("MCP response id {id} did not match expected id {expected_id}");
        }
        return ensure_jsonrpc_success(&response, expected_id).cloned();
    }
}

fn mcp_tools_from_list_result(
    server: &ConfiguredMcpServerConfig,
    result: &Value,
) -> Result<Vec<DiscoveredMcpTool>> {
    let Some(Value::Array(items)) = result.get("tools") else {
        return Ok(Vec::new());
    };
    items
        .iter()
        .map(|item| {
            let remote_tool = item
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| anyhow!("MCP tools/list item missing non-empty name"))?
                .to_owned();
            let local_name = discovered_mcp_tool_name(&server.name, &remote_tool);
            let description = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or(remote_tool.as_str());
            let input_schema = item
                .get("inputSchema")
                .or_else(|| item.get("input_schema"))
                .cloned()
                .unwrap_or_else(default_tool_input_schema_value);
            let metadata = json!({
                "mcp_server": server.name,
                "remote_tool": remote_tool,
                "discovered": true,
                "server_metadata": server.metadata,
            });
            let spec = ToolSpec::new(
                local_name,
                description,
                input_schema,
                effective_mcp_safety(server.safety.clone()),
            )
            .with_metadata(metadata);
            let spec = if let Some(timeout_ms) = server.timeout_ms {
                spec.with_timeout(timeout_ms)
            } else {
                spec
            };
            Ok(DiscoveredMcpTool { remote_tool, spec })
        })
        .collect()
}

fn next_mcp_cursor(result: &Value) -> Option<String> {
    result
        .get("nextCursor")
        .or_else(|| result.get("next_cursor"))
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(ToOwned::to_owned)
}

fn discovered_mcp_tool_name(server: &str, remote_tool: &str) -> String {
    format!(
        "{}__{}",
        sanitize_tool_name_part(server),
        sanitize_tool_name_part(remote_tool)
    )
}

fn sanitize_tool_name_part(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "mcp".to_owned()
    } else {
        sanitized
    }
}

fn effective_mcp_safety(safety: ToolSafety) -> ToolSafety {
    max_tool_safety(safety, ToolSafety::RunsCommands)
}

fn default_tool_input_schema_value() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true
    })
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
    .with_surface(configured.surface.clone())
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
        ConfiguredToolExecutorConfig::Mcp { .. } => effective_mcp_safety(configured.safety.clone()),
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

fn sync_write_json_line<W>(writer: &mut W, message: Value) -> Result<()>
where
    W: Write,
{
    writer.write_all(message.to_string().as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn sync_read_json_line<R>(reader: &mut R) -> Result<Value>
where
    R: BufRead,
{
    let mut line = Vec::with_capacity(MCP_STDIO_RESPONSE_LIMIT_BYTES.min(8192));
    loop {
        let buffer = reader.fill_buf()?;
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
        reader.consume(bytes_to_take);

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

#[cfg(test)]
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
        let mut stdout = tokio::io::BufReader::new(&response[..]);

        let error = read_json_line(&mut stdout)
            .await
            .expect_err("oversized MCP response should fail");

        assert!(
            error
                .to_string()
                .contains("MCP response exceeded 20000 bytes before newline")
        );
    }

    #[test]
    fn sync_mcp_json_line_rejects_oversized_response_without_newline() {
        let response = vec![b' '; MCP_STDIO_RESPONSE_LIMIT_BYTES + 1];
        let mut stdout = StdBufReader::new(&response[..]);

        let error =
            sync_read_json_line(&mut stdout).expect_err("oversized MCP response should fail");

        assert!(
            error
                .to_string()
                .contains("MCP response exceeded 20000 bytes before newline")
        );
    }

    #[test]
    fn mcp_discovery_times_out_when_server_is_silent() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let server = ConfiguredMcpServerConfig {
            name: "silent".to_owned(),
            command: "sh".to_owned(),
            args: vec!["-c".to_owned(), "sleep 5".to_owned()],
            protocol_version: "2024-11-05".to_owned(),
            safety: ToolSafety::ReadOnly,
            timeout_ms: Some(100),
            metadata: Value::Null,
        };
        let started = std::time::Instant::now();

        let error =
            discover_mcp_tools(&server, cwd.path()).expect_err("silent MCP server must time out");

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(error.to_string().contains("within 100ms"), "{error}");
    }
}
