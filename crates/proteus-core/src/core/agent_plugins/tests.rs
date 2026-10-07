use super::*;

#[test]
fn portable_manifest_and_mcp_keep_independent_failure_boundaries() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("plugin");
    std::fs::create_dir_all(root.join("skills")).unwrap();
    std::fs::write(root.join("plugin.json"),serde_json::json!({"$schema":manifest::PLUGIN_SCHEMA,"name":"test.plugin","unknown":"ignored","extensions":{"org.other":17}}).to_string()).unwrap();
    std::fs::write(root.join("mcp.json"),serde_json::json!({"$schema":mcp::MCP_SCHEMA,"mcpServers":{
        "valid":{"type":"stdio","command":"python3","args":["${PLUGIN_ROOT}/server.py"],"env":{"DATA":"${PLUGIN_DATA}"}},
        "bad":{"type":"stdio","command":"../escape"},
        "remote":{"type":"streamable-http","url":"https://example.com/mcp"}
    }}).to_string()).unwrap();
    let config = AddonConfig {
        disabled_mcp_servers: vec![],
        disabled_skills: vec![],
        plugins: vec![crate::domain::AgentPluginConfig {
            path: root.clone(),
            enabled: true,
        }],
    };
    let result = resolve(&config, workspace.path());
    assert!(result.plugins[0].error.is_none());
    assert_eq!(result.skills.packages.len(), 1);
    assert_eq!(result.servers.len(), 1);
    assert_eq!(result.servers[0].name, "test.plugin:valid");
    assert_eq!(result.servers[0].cwd.as_deref(), Some(root.as_path()));
    assert!(
        result.plugins[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("invalid MCP server"))
    );
    assert!(
        result.plugins[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("unsupported MCP transport"))
    );
}

#[test]
fn placeholder_expansion_is_non_recursive() {
    assert_eq!(
        paths::expand(
            "${PLUGIN_ROOT}/${OTHER}/${PLUGIN_DATA}",
            Path::new("/root/${PLUGIN_DATA}"),
            Path::new("/data")
        ),
        "/root/${PLUGIN_DATA}/${OTHER}//data"
    );
}
