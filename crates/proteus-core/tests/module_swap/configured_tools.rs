use super::*;

#[test]
fn tool_registry_rejects_duplicate_names() {
    let mut registry = ToolRegistry::new();
    registry
        .register(SearchTool::new(Arc::new(NullSearch)))
        .unwrap();

    let error = registry
        .register(SearchTool::new(Arc::new(NullSearch)))
        .unwrap_err();

    assert!(error.to_string().contains("duplicate tool registration"));
}

#[test]
fn tool_registry_tracks_tool_source() {
    let mut registry = ToolRegistry::new();
    registry
        .register_with_source(
            ToolSource::Mcp {
                server: "filesystem".to_owned(),
            },
            SearchTool::new(Arc::new(NullSearch)),
        )
        .unwrap();

    let entry = registry.entry("search").unwrap();

    assert_eq!(
        entry.source,
        ToolSource::Mcp {
            server: "filesystem".to_owned()
        }
    );
}

#[test]
fn tool_specs_are_returned_in_stable_name_order() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = registry_from_test_config(&config, dir.path());
    let names = registry
        .tools
        .specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "apply_patch",
            "remember_fact",
            "request_user_input",
            "search"
        ]
    );
}

#[tokio::test]
async fn configured_native_tool_uses_config_spec_and_native_handler() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.tools.configured.push(ConfiguredToolConfig {
        name: "project_search".to_owned(),
        description: "Configured search tool".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        }),
        surface: ToolSurface::default(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: json!({ "from": "config" }),
        executor: ConfiguredToolExecutorConfig::Native {
            handler: "search".to_owned(),
        },
    });
    let registry = registry_from_test_config(&config, dir.path());
    let spec = registry.tools.spec("project_search").unwrap();
    assert_eq!(spec.description, "Configured search tool");
    assert_eq!(spec.metadata["from"], "config");
    assert_eq!(
        registry.tools.entry("project_search").unwrap().source,
        ToolSource::Config {
            origin: "config:native".to_owned()
        }
    );
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Plan,
    );

    // ReadOnly tool under Plan mode — execution is allowed; search returns
    // an empty result (NullSearch) but that is fine for the wiring check.
    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("probe".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "project_search".to_owned(),
                json!({ "query": "anything" }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
}

#[test]
fn configured_native_tool_cannot_lower_handler_safety() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    // Handler apply_patch is WritesFiles by definition; a config that tries
    // to relabel it as ReadOnly must not be honoured.
    config.tools.configured.push(ConfiguredToolConfig {
        name: "safe_patch".to_owned(),
        description: "Mislabelled patch tool".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "patch": { "type": "string" }
            },
            "required": ["patch"]
        }),
        surface: ToolSurface::default(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Native {
            handler: "apply_patch".to_owned(),
        },
    });
    let registry = registry_from_test_config(&config, dir.path());

    assert_eq!(
        registry.tools.spec("safe_patch").unwrap().safety,
        ToolSafety::WritesFiles
    );
}

#[tokio::test]
async fn configured_process_tool_executes_through_orchestrator() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.tools.configured.push(ConfiguredToolConfig {
        name: "echo_args".to_owned(),
        description: "Echo JSON tool args from stdin".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        }),
        surface: ToolSurface::default(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Process {
            command: "sh".to_owned(),
            args: vec!["-lc".to_owned(), "cat".to_owned()],
        },
    });
    config.modules.policy = "allow_all".to_owned();
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    assert_eq!(
        registry.tools.spec("echo_args").unwrap().safety,
        ToolSafety::RunsCommands
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("echo".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "echo_args".to_owned(),
                json!({ "message": "hello" }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(result.output, "{\"message\":\"hello\"}");
    let events = events.events().await;
    assert!(matches!(events[0], Event::ToolCallRequested { .. }));
    assert!(matches!(events[1], Event::ToolFinished { .. }));
}

#[tokio::test]
async fn configured_process_tool_still_obeys_permission_mode() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.tools.configured.push(ConfiguredToolConfig {
        name: "touch_marker".to_owned(),
        description: "Create a marker file".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
        surface: ToolSurface::default(),
        safety: ToolSafety::RunsCommands,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Process {
            command: "sh".to_owned(),
            args: vec![
                "-lc".to_owned(),
                "touch should_not_exist_from_config_tool".to_owned(),
            ],
        },
    });
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Plan,
    );
    let orchestrator = ToolOrchestrator::default();

    assert!(
        orchestrator
            .visible_tool_specs(&ctx, dir.path())
            .into_iter()
            .all(|spec| spec.name != "touch_marker")
    );

    let result = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("touch".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "touch_marker".to_owned(), json!({})),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("permission mode plan"))
    );
    assert!(
        !dir.path()
            .join("should_not_exist_from_config_tool")
            .exists()
    );
}

#[tokio::test]
async fn configured_mcp_server_discovers_tools_into_registry() {
    let dir = temp_workspace();
    let server = dir.path().join("mcp_discovery_server.sh");
    std::fs::write(
        &server,
        r#"#!/bin/sh
calls=0
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}\n' "$id"
      ;;
    *'"method":"notifications/initialized"'*)
      ;;
    *'"method":"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"remote_echo","description":"Remote echo","inputSchema":{"type":"object","properties":{"message":{"type":"string"}}}}]}}\n' "$id"
      ;;
    *'"method":"tools/call"'*)
      case "$line" in
        *'"name":"remote_echo"'*)
          calls=$((calls + 1))
          printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"discovered-%s"}],"isError":false}}\n' "$id" "$calls"
          ;;
        *)
          printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"wrong tool"}}\n' "$id"
          ;;
      esac
      ;;
  esac
done
"#,
    )
    .unwrap();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.modules.policy = "allow_all".to_owned();
    config.tools.mcp_servers.push(ConfiguredMcpServerConfig {
        name: "demo-mcp".to_owned(),
        command: "sh".to_owned(),
        args: vec![server.to_string_lossy().to_string()],
        protocol_version: "2025-06-18".to_owned(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        max_response_bytes: None,
        metadata: json!({ "scope": "test" }),
    });

    let registry = registry_from_test_config(&config, dir.path());
    let spec = registry.tools.spec("demo-mcp__remote_echo").unwrap();
    assert_eq!(spec.description, "Remote echo");
    assert_eq!(spec.safety, ToolSafety::RunsCommands);
    assert_eq!(spec.metadata["mcp_server"], "demo-mcp");
    assert_eq!(spec.metadata["remote_tool"], "remote_echo");
    assert_eq!(
        registry
            .tools
            .entry("demo-mcp__remote_echo")
            .unwrap()
            .source,
        ToolSource::Mcp {
            server: "demo-mcp".to_owned()
        }
    );

    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    let orchestrator = ToolOrchestrator::default();
    let result = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("mcp".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "demo-mcp__remote_echo".to_owned(),
                json!({ "message": "hello" }),
            ),
        )
        .await
        .unwrap();
    let second = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("mcp".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "demo-mcp__remote_echo".to_owned(),
                json!({ "message": "again" }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(result.output, "discovered-1");
    assert_eq!(second.output, "discovered-2");
    assert_eq!(result.metadata["remote_tool"], "remote_echo");
}

#[tokio::test]
async fn configured_mcp_tool_executes_fixed_remote_tool_through_orchestrator() {
    let dir = temp_workspace();
    let server = dir.path().join("mcp_server.sh");
    std::fs::write(
        &server,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}'
      ;;
    *'"method":"notifications/initialized"'*)
      ;;
    *'"method":"tools/call"'*)
      case "$line" in
        *'"name":"remote_echo"'*)
          printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"mcp ok"}],"isError":false}}'
          ;;
        *)
          printf '%s\n' '{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"wrong tool"}}'
          ;;
      esac
      ;;
  esac
done
"#,
    )
    .unwrap();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.modules.policy = "allow_all".to_owned();
    config.tools.configured.push(ConfiguredToolConfig {
        name: "mcp_echo".to_owned(),
        description: "Call a fixed MCP echo tool".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
        surface: ToolSurface::default(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Mcp {
            server: Some("test-mcp".to_owned()),
            command: "sh".to_owned(),
            args: vec![server.to_string_lossy().to_string()],
            tool: "remote_echo".to_owned(),
            protocol_version: "2025-06-18".to_owned(),
            max_response_bytes: None,
        },
    });
    let registry = registry_from_test_config(&config, dir.path());
    assert_eq!(
        registry.tools.spec("mcp_echo").unwrap().safety,
        ToolSafety::RunsCommands
    );
    assert_eq!(
        registry.tools.entry("mcp_echo").unwrap().source,
        ToolSource::Mcp {
            server: "test-mcp".to_owned()
        }
    );
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask::new("mcp".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(
                new_call_id(),
                "mcp_echo".to_owned(),
                json!({ "name": "attempted_override" }),
            ),
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(result.output, "mcp ok");
    assert_eq!(result.metadata["executor"], "mcp");
    assert_eq!(result.metadata["remote_tool"], "remote_echo");
    let events = events.events().await;
    assert!(matches!(events[0], Event::ToolCallRequested { .. }));
    assert!(matches!(events[1], Event::ToolFinished { .. }));
}

#[tokio::test]
async fn configured_mcp_tool_reuses_stdio_session_between_calls() {
    let dir = temp_workspace();
    let server = dir.path().join("mcp_persistent_server.sh");
    std::fs::write(
        &server,
        r#"#!/bin/sh
calls=0
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}\n' "$id"
      ;;
    *'"method":"notifications/initialized"'*)
      ;;
    *'"method":"tools/call"'*)
      calls=$((calls + 1))
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"call-%s"}],"isError":false}}\n' "$id" "$calls"
      ;;
  esac
done
"#,
    )
    .unwrap();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.modules.policy = "allow_all".to_owned();
    config.tools.configured.push(ConfiguredToolConfig {
        name: "mcp_counter".to_owned(),
        description: "Call a persistent MCP counter tool".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
        surface: ToolSurface::default(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Mcp {
            server: Some("counter-mcp".to_owned()),
            command: "sh".to_owned(),
            args: vec![server.to_string_lossy().to_string()],
            tool: "counter".to_owned(),
            protocol_version: "2025-06-18".to_owned(),
            max_response_bytes: None,
        },
    });
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Normal,
    );
    let orchestrator = ToolOrchestrator::default();

    let first = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("mcp".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "mcp_counter".to_owned(), json!({})),
        )
        .await
        .unwrap();
    let second = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("mcp".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "mcp_counter".to_owned(), json!({})),
        )
        .await
        .unwrap();

    assert_eq!(first.output, "call-1");
    assert_eq!(second.output, "call-2");
}

#[tokio::test]
async fn configured_mcp_tool_still_obeys_permission_mode() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    clear_ask_write_config(&mut config);
    config.tools.configured.push(ConfiguredToolConfig {
        name: "mcp_hidden".to_owned(),
        description: "Hidden MCP command tool".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
        surface: ToolSurface::default(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Mcp {
            server: Some("hidden-mcp".to_owned()),
            command: "sh".to_owned(),
            args: vec!["-c".to_owned(), "exit 99".to_owned()],
            tool: "remote".to_owned(),
            protocol_version: "2025-06-18".to_owned(),
            max_response_bytes: None,
        },
    });
    let registry = registry_from_test_config(&config, dir.path());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events)),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Plan,
    );
    let orchestrator = ToolOrchestrator::default();

    assert!(
        orchestrator
            .visible_tool_specs(&ctx, dir.path())
            .into_iter()
            .all(|spec| spec.name != "mcp_hidden")
    );

    let result = orchestrator
        .execute(
            &ctx,
            &AgentTask::new("mcp".to_owned(), dir.path().to_path_buf()),
            ToolCall::new(new_call_id(), "mcp_hidden".to_owned(), json!({})),
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("permission mode plan"))
    );
}
