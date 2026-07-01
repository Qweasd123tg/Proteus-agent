use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    contracts::{Tool, ToolContext, ToolRegistry, ToolSource},
    core::ConfiguredMcpServerConfig,
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
};

mod discovery;
mod protocol;
mod session;

use discovery::DiscoveredMcpTool;
use protocol::render_mcp_content;
use session::McpStdioSession;

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

#[derive(Debug)]
pub(super) struct McpStdioHost {
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

pub(super) fn configured_mcp_inline_host(
    server_name: String,
    command: String,
    args: Vec<String>,
    protocol_version: String,
    cwd: &Path,
    timeout_ms: u64,
) -> Arc<McpStdioHost> {
    Arc::new(McpStdioHost::new(
        server_name,
        command,
        args,
        protocol_version,
        cwd.to_path_buf(),
        Duration::from_millis(timeout_ms),
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
        server.name.clone(),
        server.command.clone(),
        server.args.clone(),
        server.protocol_version.clone(),
        cwd.to_path_buf(),
        Duration::from_millis(server.timeout_ms.unwrap_or(30_000)),
    ))
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
