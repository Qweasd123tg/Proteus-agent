use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use async_trait::async_trait;
use proteus_process_host::ProcessSpec;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};

use crate::{
    contracts::{Tool, ToolContext, ToolRegistry, ToolSource},
    core::{ConfiguredMcpServerConfig, ProcessEnvironmentConfig},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

mod client;
mod discovery;
mod transport;

pub(super) use client::McpStdioHost;

#[cfg(test)]
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

        let result = host
            .call_tool(remote_tool, args, timeout, ctx.cancellation)
            .await?;
        let is_error = result.is_error.unwrap_or(false);
        let content_text = render_mcp_content(&result.content)?;
        let error = is_error.then(|| content_text.clone());
        let metadata = json!({
            "tool": call.name,
            "executor": "mcp",
            "remote_tool": self.remote_tool,
            "structured_content": result.structured_content.unwrap_or(Value::Null),
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

pub(super) fn configured_mcp_inline_host(
    command: String,
    args: Vec<String>,
    environment: ProcessEnvironmentConfig,
    protocol_version: String,
    cwd: &Path,
    timeout_ms: u64,
    max_response_bytes: Option<usize>,
) -> Result<Arc<McpStdioHost>> {
    let spec = process_spec(command, args, environment, cwd);
    Ok(Arc::new(McpStdioHost::new(
        spec,
        protocol_version,
        Duration::from_millis(timeout_ms),
        max_response_bytes
            .unwrap_or(crate::core::process_output::DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES),
    )?))
}

pub(super) fn register_discovered_mcp_tools(
    registry: &mut ToolRegistry,
    mcp_servers: &[ConfiguredMcpServerConfig],
    cwd: &Path,
) -> Result<()> {
    for server in mcp_servers {
        let host = configured_mcp_server_host(server, cwd)?;
        let discovered = discovery::mcp_tools_from_list(server, host.list_tools()?)?;
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

fn configured_mcp_server_host(
    server: &ConfiguredMcpServerConfig,
    cwd: &Path,
) -> Result<Arc<McpStdioHost>> {
    let spec = process_spec(
        server.command.clone(),
        server.args.clone(),
        server.environment.clone(),
        cwd,
    );
    Ok(Arc::new(McpStdioHost::new(
        spec,
        server.protocol_version.clone(),
        Duration::from_millis(server.timeout_ms.unwrap_or(30_000)),
        server
            .max_response_bytes
            .unwrap_or(crate::core::process_output::DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES),
    )?))
}

fn process_spec(
    command: String,
    args: Vec<String>,
    environment: ProcessEnvironmentConfig,
    cwd: &Path,
) -> ProcessSpec {
    ProcessSpec::new(command)
        .args(args)
        .env_allowlist(environment.env_allowlist)
        .envs(environment.env)
        .cwd(cwd)
}

fn render_mcp_content(items: &[ContentBlock]) -> Result<String> {
    items
        .iter()
        .map(|item| match item.as_text() {
            Some(text) => Ok(text.text.clone()),
            None => serde_json::to_string(item).map_err(Into::into),
        })
        .collect::<Result<Vec<_>>>()
        .map(|items| items.join("\n"))
}

pub(super) fn effective_mcp_safety(safety: ToolSafety) -> ToolSafety {
    discovery::effective_mcp_safety(safety)
}

#[cfg(test)]
fn discover_mcp_tools(
    server: &ConfiguredMcpServerConfig,
    cwd: &Path,
) -> Result<Vec<DiscoveredMcpTool>> {
    discovery::mcp_tools_from_list(
        server,
        configured_mcp_server_host(server, cwd)?.list_tools()?,
    )
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
            environment: ProcessEnvironmentConfig::default(),
            protocol_version: "2024-11-05".to_owned(),
            safety: ToolSafety::ReadOnly,
            supports_parallel_tool_calls: false,
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
