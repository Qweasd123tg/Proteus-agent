use std::{
    path::Path,
    process::{Child as StdChild, ChildStdin as StdChildStdin, Command as StdCommand, Stdio},
    sync::mpsc::Receiver,
    time::Duration,
};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::core::ConfiguredMcpServerConfig;

use super::{
    discovery::{DiscoveredMcpTool, mcp_tools_from_list_result, next_mcp_cursor},
    protocol::{recv_sync_jsonrpc_success, spawn_sync_json_line_reader, sync_write_json_line},
};

pub(super) struct McpStdioSession {
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
    pub(super) fn start(
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

    pub(super) fn list_tools(
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

    pub(super) fn call_tool(
        &mut self,
        remote_tool: &str,
        args: Value,
        timeout: Duration,
    ) -> Result<Value> {
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
