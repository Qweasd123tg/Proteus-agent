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

pub(super) fn mcp_tools_from_list(
    server: &ConfiguredMcpServerConfig,
    tools: Vec<rmcp::model::Tool>,
) -> Result<Vec<DiscoveredMcpTool>> {
    tools
        .into_iter()
        .map(|tool| {
            let remote_tool = tool.name.into_owned();
            if remote_tool.trim().is_empty() {
                return Err(anyhow!("MCP tools/list item missing non-empty name"));
            }
            let local_name = discovered_mcp_tool_name(&server.name, &remote_tool);
            let description = tool.description.as_deref().unwrap_or(remote_tool.as_str());
            let input_schema = Value::Object((*tool.input_schema).clone());
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
            .with_parallel_tool_calls(
                server.supports_parallel_tool_calls
                    || tool
                        .annotations
                        .as_ref()
                        .and_then(|annotations| annotations.read_only_hint)
                        == Some(true),
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

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{JsonObject, Tool, ToolAnnotations};

    fn tool(name: &'static str, read_only_hint: Option<bool>) -> Tool {
        let mut schema = JsonObject::new();
        schema.insert("type".to_owned(), json!("object"));
        let tool = Tool::new(name, format!("{name} description"), schema);
        match read_only_hint {
            Some(read_only) => tool.with_annotations(ToolAnnotations::new().read_only(read_only)),
            None => tool,
        }
    }

    #[test]
    fn parallel_permission_comes_from_config_or_annotation_without_lowering_safety() {
        let mut server: ConfiguredMcpServerConfig = serde_json::from_value(json!({
            "name": "probe", "command": "unused", "safety": "ReadOnly"
        }))
        .unwrap();
        let tools = vec![
            tool("read", Some(true)),
            tool("other", Some(false)),
            tool("unannotated", None),
        ];
        let discovered = mcp_tools_from_list(&server, tools.clone()).unwrap();
        assert_eq!(
            discovered
                .iter()
                .map(|tool| tool.spec.supports_parallel_tool_calls)
                .collect::<Vec<_>>(),
            [true, false, false]
        );
        assert!(
            discovered
                .iter()
                .all(|tool| tool.spec.safety == ToolSafety::RunsCommands)
        );
        server.supports_parallel_tool_calls = true;
        assert!(
            mcp_tools_from_list(&server, tools)
                .unwrap()
                .iter()
                .all(|tool| tool.spec.supports_parallel_tool_calls)
        );
    }

    #[test]
    fn sdk_requires_and_preserves_the_canonical_input_schema() {
        assert!(
            serde_json::from_value::<Tool>(json!({
                "name": "missing-schema",
                "description": "invalid MCP tool"
            }))
            .is_err()
        );

        let mut schema = JsonObject::new();
        schema.insert("type".to_owned(), json!("object"));
        schema.insert("required".to_owned(), json!(["path"]));
        schema.insert("properties".to_owned(), json!({"path": {"type": "string"}}));
        let server: ConfiguredMcpServerConfig = serde_json::from_value(json!({
            "name": "probe", "command": "unused"
        }))
        .unwrap();

        let discovered = mcp_tools_from_list(
            &server,
            vec![Tool::new_with_raw("read", None, schema.clone())],
        )
        .unwrap();
        assert_eq!(discovered[0].spec.input_schema, Value::Object(schema));
        assert_eq!(discovered[0].spec.description, "read");
    }

    #[test]
    fn discovery_preserves_remote_identity_and_server_options() {
        let server: ConfiguredMcpServerConfig = serde_json::from_value(json!({
            "name": "probe server",
            "command": "unused",
            "timeout_ms": 750,
            "metadata": {"owner": "test"}
        }))
        .unwrap();
        let remote_tool = "read file/原";

        let discovered = mcp_tools_from_list(
            &server,
            vec![Tool::new(remote_tool, "Read a file", JsonObject::new())],
        )
        .unwrap();
        let tool = &discovered[0];

        assert_eq!(tool.remote_tool, remote_tool);
        assert_eq!(tool.spec.name, "probe_server__read_file__");
        assert_eq!(tool.spec.description, "Read a file");
        assert_eq!(tool.spec.timeout_ms, Some(750));
        assert_eq!(
            tool.spec.metadata,
            json!({
                "mcp_server": "probe server",
                "remote_tool": remote_tool,
                "discovered": true,
                "server_metadata": {"owner": "test"}
            })
        );

        let error = mcp_tools_from_list(
            &server,
            vec![Tool::new("   ", "invalid", JsonObject::new())],
        )
        .unwrap_err();
        assert!(error.to_string().contains("non-empty name"));
    }
}
