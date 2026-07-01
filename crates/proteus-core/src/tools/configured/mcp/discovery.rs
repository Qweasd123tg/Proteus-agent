use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::{
    core::ConfiguredMcpServerConfig,
    domain::{ToolSafety, ToolSpec},
};

#[derive(Debug)]
pub(super) struct DiscoveredMcpTool {
    pub(super) remote_tool: String,
    pub(super) spec: ToolSpec,
}

pub(super) fn mcp_tools_from_list_result(
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

pub(super) fn next_mcp_cursor(result: &Value) -> Option<String> {
    result
        .get("nextCursor")
        .or_else(|| result.get("next_cursor"))
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn effective_mcp_safety(safety: ToolSafety) -> ToolSafety {
    super::super::max_tool_safety(safety, ToolSafety::RunsCommands)
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

fn default_tool_input_schema_value() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true
    })
}
