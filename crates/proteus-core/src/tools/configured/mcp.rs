use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use async_trait::async_trait;
use proteus_process_host::{NewlineJsonFraming, ProcessHost, ProcessSpec};
use serde_json::{Value, json};

use crate::{
    contracts::{Tool, ToolContext, ToolRegistry, ToolSource},
    core::ConfiguredMcpServerConfig,
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

mod discovery;

use discovery::DiscoveredMcpTool;

#[derive(Clone)]
pub struct ConfiguredMcpTool {
    spec: ToolSpec,
    remote_tool: String,
    host: Arc<McpStdioHost>,
}

impl ConfiguredMcpTool {
    pub(super) fn new(spec: ToolSpec, remote_tool: String, host: Arc<McpStdioHost>) -> Self {
        Self {
            spec,
            remote_tool,
            host,
        }
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

/// Persistent stdio MCP server behind the shared process host: lazy start,
/// `initialize` handshake on every (re)spawn, restart on next use after errors.
#[derive(Debug)]
pub(super) struct McpStdioHost {
    timeout: Duration,
    host: ProcessHost<NewlineJsonFraming>,
}

impl McpStdioHost {
    fn new(
        command: String,
        args: Vec<String>,
        protocol_version: String,
        cwd: &Path,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Self {
        let spec = ProcessSpec::new(command).args(args).cwd(cwd);
        let framing = NewlineJsonFraming::new(max_response_bytes);
        let host = ProcessHost::with_initializer(spec, framing, move |session| {
            session.request(
                "initialize",
                json!({
                    "protocolVersion": protocol_version,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "proteus-core",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
                timeout,
            )?;
            session.notify("notifications/initialized", json!({}))
        });
        Self { timeout, host }
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    fn call_tool(&self, remote_tool: &str, args: Value, timeout: Duration) -> Result<Value> {
        let result = self.host.request(
            "tools/call",
            json!({
                "name": remote_tool,
                "arguments": args
            }),
            timeout,
        );
        // MCP notifications are not consumed anywhere yet; drop them so a
        // chatty server does not grow the session buffer unboundedly.
        self.host.drain_notifications();
        result
    }

    fn list_tools(&self, server: &ConfiguredMcpServerConfig) -> Result<Vec<DiscoveredMcpTool>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = cursor
                .as_ref()
                .map(|cursor| json!({ "cursor": cursor }))
                .unwrap_or_else(|| json!({}));
            let result = self.host.request("tools/list", params, self.timeout);
            self.host.drain_notifications();
            let result = result?;
            tools.extend(discovery::mcp_tools_from_list_result(server, &result)?);
            cursor = discovery::next_mcp_cursor(&result);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }
}

pub(super) fn configured_mcp_inline_host(
    command: String,
    args: Vec<String>,
    protocol_version: String,
    cwd: &Path,
    timeout_ms: u64,
    max_response_bytes: Option<usize>,
) -> Arc<McpStdioHost> {
    Arc::new(McpStdioHost::new(
        command,
        args,
        protocol_version,
        cwd,
        Duration::from_millis(timeout_ms),
        max_response_bytes
            .unwrap_or(crate::core::process_output::DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES),
    ))
}

pub(super) fn register_discovered_mcp_tools(
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
        server.command.clone(),
        server.args.clone(),
        server.protocol_version.clone(),
        cwd,
        Duration::from_millis(server.timeout_ms.unwrap_or(30_000)),
        server
            .max_response_bytes
            .unwrap_or(crate::core::process_output::DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES),
    ))
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

pub(super) fn effective_mcp_safety(safety: ToolSafety) -> ToolSafety {
    discovery::effective_mcp_safety(safety)
}

#[cfg(test)]
fn discover_mcp_tools(
    server: &ConfiguredMcpServerConfig,
    cwd: &Path,
) -> Result<Vec<DiscoveredMcpTool>> {
    configured_mcp_server_host(server, cwd).list_tools(server)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use crate::domain::ToolSafety;

    use super::*;

    #[test]
    fn mcp_discovery_times_out_when_server_is_silent() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let server = ConfiguredMcpServerConfig {
            max_response_bytes: None,
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
