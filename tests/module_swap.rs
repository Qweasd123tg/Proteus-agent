use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use modular_agent::{
    contracts::{
        ApprovalRequest, ApprovalResponse, ApprovalTransport, ContextBuildInput, EventEmitter,
        ModelAdapter, ModelClient, PolicyContext, Tool, ToolContext, ToolRegistry, ToolSource,
        Workflow,
    },
    core::{
        AgentRuntime, AppConfig, BuiltinModuleCatalog, BuiltinRegistry, ConfiguredToolConfig,
        ConfiguredToolExecutorConfig, FanoutEventSink, InMemoryEventStore, ToolOrchestrator,
    },
    domain::{
        AgentTask, CacheHints, Event, EventContext, ModelLimits, ModelRef, ModuleKind,
        PermissionMode, PolicyDecision, ReasoningConfig, ResponseFormat, SamplingConfig, ToolCall,
        ToolChoice, ToolResult, ToolSafety, ToolSpec, new_call_id, new_session_id, new_thread_id,
        new_turn_id,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities,
    },
    modules::{
        ApplyPatchTool, DirectPatchApplier, FakeModelClient, ListDirTool, ModelService, NoMemory,
        NullSearch, ReadFileTool, SingleLoopWorkflow, WriteFileTool,
    },
};
use serde_json::json;
use tempfile::TempDir;

fn temp_workspace() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("sample.txt"), "hello modular agent\n").expect("sample file");
    dir
}

async fn run_with(config: AppConfig, task: &str) -> (String, Arc<InMemoryEventStore>) {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime =
        AgentRuntime::with_event_sink(config, dir.path().to_path_buf(), events.clone()).unwrap();
    let output = runtime.run(task.to_owned()).await.unwrap();
    (output.text, events)
}

fn test_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.tools.enabled = standard_tool_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    config.tools.path = None;
    config.policy.ask_write.ask_before = ["apply_patch", "write_file", "shell"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    config.policy.ask_write.allow = ["read_file", "list_dir", "search"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    config
}

fn configured_tool_names(config: &AppConfig) -> Vec<&str> {
    let mut names = config
        .tools
        .configured
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn standard_tool_names() -> Vec<&'static str> {
    let mut names = vec![
        "read_file",
        "list_dir",
        "apply_patch",
        "write_file",
        "shell",
        "search",
    ];
    names.sort();
    names
}

#[test]
fn builtin_module_catalog_lists_builtin_slots() {
    let catalog = BuiltinModuleCatalog::new();

    let model_ids = catalog
        .manifests_by_kind(ModuleKind::Model)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let search_ids = catalog
        .manifests_by_kind(ModuleKind::Search)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let memory_policy_ids = catalog
        .manifests_by_kind(ModuleKind::MemoryPolicy)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();
    let context_ids = catalog
        .manifests_by_kind(ModuleKind::Context)
        .into_iter()
        .map(|manifest| manifest.id)
        .collect::<Vec<_>>();

    assert_eq!(
        model_ids,
        ["anthropic", "fake", "openai", "openai_compatible"]
    );
    assert_eq!(search_ids, ["null", "rg"]);
    assert_eq!(memory_policy_ids, ["none"]);
    assert_eq!(context_ids, ["repo_aware", "simple"]);
    assert_eq!(
        catalog
            .manifest(ModuleKind::Workflow, "single_loop")
            .unwrap()
            .capabilities,
        ["model", "tools", "context"]
    );
    assert!(catalog.manifest(ModuleKind::Tool, "read_file").is_none());
}

#[tokio::test]
async fn statusline_renderer_composes_configured_components() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.renderer = "statusline".to_owned();
    config.renderer.statusline.components = vec!["model".to_owned(), "context".to_owned()];
    config.renderer.statusline.ansi = false;
    config.renderer.statusline.context.max_tokens = Some(100);

    let events = Arc::new(InMemoryEventStore::new());
    let runtime =
        AgentRuntime::with_event_sink(config, dir.path().to_path_buf(), events.clone()).unwrap();
    let output = runtime.run("summarize hello".to_owned()).await.unwrap();
    let rendered = runtime.render(&output).await.unwrap();

    assert!(rendered.contains("model fake/fake-tool-model"));
    assert!(rendered.contains("ctx ["));
    assert!(rendered.contains("Fake final answer"));
}

#[test]
fn statusline_renderer_rejects_unknown_component() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.renderer = "statusline".to_owned();
    config.renderer.statusline.components = vec!["unknown".to_owned()];

    let error = match BuiltinRegistry::from_config(&config, dir.path().to_path_buf()) {
        Ok(_) => panic!("unknown statusline component should be rejected"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("unsupported statusline component: unknown")
    );
}

#[test]
fn statusline_renderer_rejects_unknown_position_at_startup() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.renderer = "statusline".to_owned();
    config.renderer.statusline.position = "middle".to_owned();

    let error = match BuiltinRegistry::from_config(&config, dir.path().to_path_buf()) {
        Ok(_) => panic!("unknown statusline position should be rejected"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("unsupported statusline position: middle")
    );
}

#[test]
fn statusline_renderer_rejects_unknown_frame_at_startup() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.renderer = "statusline".to_owned();
    config.renderer.statusline.frame = "floating".to_owned();

    let error = match BuiltinRegistry::from_config(&config, dir.path().to_path_buf()) {
        Ok(_) => panic!("unknown statusline frame should be rejected"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("unsupported statusline frame: floating")
    );
}

#[tokio::test]
async fn swapping_context_builder_does_not_change_runtime() {
    for context in ["simple", "repo_aware"] {
        let mut config = test_config();
        config.modules.context = context.to_owned();
        config.context.repo_aware.providers = vec!["repo_tree".to_owned()];

        let (output, events) = run_with(config, "summarize context").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[test]
fn unknown_repo_aware_provider_is_rejected_at_startup() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    config.context.repo_aware.providers = vec!["mystery".to_owned()];

    let error = match BuiltinRegistry::from_config(&config, dir.path().to_path_buf()) {
        Ok(_) => panic!("unknown repo_aware provider should be rejected"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("unsupported repo_aware context provider: mystery")
    );
}

#[tokio::test]
async fn repo_aware_context_collects_provider_chunks_with_metadata() {
    let dir = temp_workspace();
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "Run cargo test before finishing.\n",
    )
    .expect("agents");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("manifest");
    std::fs::write(dir.path().join("src.rs"), "fn main() {}\n").expect("source");
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    config.context.repo_aware.providers = vec![
        "project_instructions".to_owned(),
        "manifest".to_owned(),
        "repo_tree".to_owned(),
    ];
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask {
                text: "summarize repo".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .unwrap();

    assert!(
        bundle
            .chunks
            .iter()
            .any(|chunk| chunk.source == "repo_aware:task")
    );
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:project_instructions"
            && chunk.path.as_deref() == Some(std::path::Path::new("AGENTS.md"))
            && chunk.content.contains("cargo test")
            && chunk.metadata["provider"] == "project_instructions"
            && chunk.metadata["reason"] == "project instruction file"
    }));
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:manifest"
            && chunk.path.as_deref() == Some(std::path::Path::new("Cargo.toml"))
            && chunk.content.contains("name = \"demo\"")
    }));
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:repo_tree" && chunk.content.contains("src.rs")
    }));
}

#[tokio::test]
async fn repo_aware_context_does_not_read_configured_paths_outside_workspace() {
    let dir = temp_workspace();
    let outside = tempfile::tempdir().expect("outside dir");
    std::fs::write(outside.path().join("secret.md"), "do not read").expect("secret");
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    config.context.repo_aware.providers = vec!["project_instructions".to_owned()];
    config.context.repo_aware.project_instruction_files = vec![format!(
        "../{}/secret.md",
        outside.path().file_name().unwrap().to_string_lossy()
    )];
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask {
                text: "summarize repo".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .unwrap();

    assert!(
        !bundle
            .chunks
            .iter()
            .any(|chunk| chunk.content.contains("do not read"))
    );
}

#[tokio::test]
async fn swapping_search_backend_does_not_change_runtime() {
    for search in ["null", "rg"] {
        let mut config = test_config();
        config.modules.search = search.to_owned();

        let (output, events) = run_with(config, "summarize hello").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn swapping_memory_store_does_not_change_runtime() {
    for memory in ["none", "jsonl"] {
        let mut config = test_config();
        config.modules.memory = memory.to_owned();

        let (output, events) = run_with(config, "summarize memory").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn no_memory_policy_does_not_write_memory_automatically() {
    let mut config = test_config();
    config.modules.memory = "jsonl".to_owned();
    config.modules.memory_policy = "none".to_owned();

    let (_output, events) = run_with(config, "remember nothing automatically").await;

    assert!(
        !events
            .events()
            .await
            .iter()
            .any(|event| matches!(event, Event::MemoryWritten { .. }))
    );
}

#[test]
fn unknown_memory_policy_is_rejected_at_startup() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.memory_policy = "auto_summary".to_owned();

    let error = match BuiltinRegistry::from_config(&config, dir.path().to_path_buf()) {
        Ok(_) => panic!("unknown memory policy should be rejected"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("unsupported memory_policy module: auto_summary")
    );
}

#[tokio::test]
async fn swapping_policy_does_not_change_read_tool_execution() {
    for policy in ["allow_all", "ask_write"] {
        let mut config = test_config();
        config.modules.policy = policy.to_owned();

        let (output, events) = run_with(config, "read_file sample.txt").await;

        assert!(output.contains("hello modular agent"));
        assert!(events.events().await.len() >= 8);
    }
}

#[tokio::test]
async fn runtime_keeps_session_id_and_creates_new_turn_id_per_run() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime =
        AgentRuntime::with_event_sink(test_config(), dir.path().to_path_buf(), events.clone())
            .unwrap();

    let first = runtime.run("summarize first".to_owned()).await.unwrap();
    let second = runtime.run("summarize second".to_owned()).await.unwrap();
    let records = events.events().await;

    let session_ids = records
        .iter()
        .filter_map(|event| match event {
            Event::SessionStarted { session_id, .. } => Some(*session_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let turn_ids = records
        .iter()
        .filter_map(|event| match event {
            Event::TurnStarted {
                session_id,
                turn_id,
                ..
            } => Some((*session_id, *turn_id)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(session_ids.len(), 1);
    assert_eq!(turn_ids.len(), 2);
    assert_eq!(turn_ids[0].0, session_ids[0]);
    assert_eq!(turn_ids[1].0, session_ids[0]);
    assert_ne!(turn_ids[0].1, turn_ids[1].1);
    assert_eq!(first.metadata["session_id"], session_ids[0].to_string());
    assert_eq!(second.metadata["session_id"], session_ids[0].to_string());
    assert_ne!(first.metadata["turn_id"], second.metadata["turn_id"]);
}

#[tokio::test]
async fn fanout_preserves_event_envelope_identity_across_sinks() {
    let left = Arc::new(InMemoryEventStore::new());
    let right = Arc::new(InMemoryEventStore::new());
    let emitter = EventEmitter::new(Arc::new(FanoutEventSink::new(vec![
        left.clone(),
        right.clone(),
    ])));
    let session_id = new_session_id();
    let thread_id = new_thread_id();

    emitter
        .emit(
            EventContext {
                session_id,
                thread_id,
                turn_id: None,
            },
            Event::Error {
                message: "same envelope".to_owned(),
            },
        )
        .await
        .unwrap();

    let left = left.envelopes().await;
    let right = right.envelopes().await;

    assert_eq!(left.len(), 1);
    assert_eq!(right.len(), 1);
    assert_eq!(left[0].event_id, right[0].event_id);
    assert_eq!(left[0].seq, 1);
    assert_eq!(right[0].seq, 1);
    assert_eq!(left[0].session_id, session_id);
    assert_eq!(left[0].thread_id, thread_id);
    assert_eq!(left[0].turn_id, None);
    assert_eq!(left[0].schema_version, 1);
}

#[tokio::test]
async fn tool_visibility_and_execution_policy_are_separate() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();

    assert!(registry.tools.spec("write_file").is_ok());

    let call = ToolCall {
        id: new_call_id(),
        name: "write_file".to_owned(),
        args: json!({ "path": "x.txt", "content": "x" }),
    };
    let decision = registry.policy.evaluate(
        &call,
        &PolicyContext {
            cwd: dir.path().to_path_buf(),
            tool_spec: registry.tools.spec("write_file").ok(),
        },
    );

    assert!(matches!(decision, PolicyDecision::Ask { .. }));
}

#[test]
fn tool_registry_rejects_duplicate_names() {
    let mut registry = ToolRegistry::new();
    registry.register(ReadFileTool).unwrap();

    let error = registry.register(ReadFileTool).unwrap_err();

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
            ReadFileTool,
        )
        .unwrap();

    let entry = registry.entry("read_file").unwrap();

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
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
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
            "list_dir",
            "read_file",
            "search",
            "shell",
            "write_file"
        ]
    );
}

#[tokio::test]
async fn configured_native_tool_uses_config_spec_and_native_handler() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    config.policy.ask_write.allow = Vec::new();
    config.policy.ask_write.ask_before = Vec::new();
    config.tools.configured.push(ConfiguredToolConfig {
        name: "read_file".to_owned(),
        description: "Configured read tool".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        }),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: json!({ "from": "config" }),
        executor: ConfiguredToolExecutorConfig::Native {
            handler: "read_file".to_owned(),
        },
    });
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    let spec = registry.tools.spec("read_file").unwrap();
    assert_eq!(spec.description, "Configured read tool");
    assert_eq!(spec.metadata["from"], "config");
    assert_eq!(
        registry.tools.entry("read_file").unwrap().source,
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

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask {
                text: "read".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            ToolCall {
                id: new_call_id(),
                name: "read_file".to_owned(),
                args: json!({ "path": "sample.txt" }),
            },
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert!(result.output.contains("hello modular agent"));
}

#[test]
fn configured_native_tool_cannot_lower_handler_safety() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    config.policy.ask_write.allow = Vec::new();
    config.policy.ask_write.ask_before = Vec::new();
    config.tools.configured.push(ConfiguredToolConfig {
        name: "shell".to_owned(),
        description: "Configured shell".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" }
            },
            "required": ["command"]
        }),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Native {
            handler: "shell".to_owned(),
        },
    });
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();

    assert_eq!(
        registry.tools.spec("shell").unwrap().safety,
        ToolSafety::RunsCommands
    );
}

#[tokio::test]
async fn configured_process_tool_executes_through_orchestrator() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    config.policy.ask_write.allow = Vec::new();
    config.policy.ask_write.ask_before = Vec::new();
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
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Process {
            command: "sh".to_owned(),
            args: vec!["-lc".to_owned(), "cat".to_owned()],
        },
    });
    config.modules.policy = "allow_all".to_owned();
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
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
            &AgentTask {
                text: "echo".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            ToolCall {
                id: new_call_id(),
                name: "echo_args".to_owned(),
                args: json!({ "message": "hello" }),
            },
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
    config.policy.ask_write.allow = Vec::new();
    config.policy.ask_write.ask_before = Vec::new();
    config.tools.configured.push(ConfiguredToolConfig {
        name: "touch_marker".to_owned(),
        description: "Create a marker file".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
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
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
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
            &AgentTask {
                text: "touch".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            ToolCall {
                id: new_call_id(),
                name: "touch_marker".to_owned(),
                args: json!({}),
            },
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
    config.policy.ask_write.allow = Vec::new();
    config.policy.ask_write.ask_before = Vec::new();
    config.modules.policy = "allow_all".to_owned();
    config.tools.configured.push(ConfiguredToolConfig {
        name: "mcp_echo".to_owned(),
        description: "Call a fixed MCP echo tool".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Mcp {
            server: Some("test-mcp".to_owned()),
            command: "sh".to_owned(),
            args: vec![server.to_string_lossy().to_string()],
            tool: "remote_echo".to_owned(),
            protocol_version: "2025-06-18".to_owned(),
        },
    });
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
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
            &AgentTask {
                text: "mcp".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            ToolCall {
                id: new_call_id(),
                name: "mcp_echo".to_owned(),
                args: json!({ "name": "attempted_override" }),
            },
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
async fn configured_mcp_tool_still_obeys_permission_mode() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.tools.enabled = Vec::new();
    config.policy.ask_write.allow = Vec::new();
    config.policy.ask_write.ask_before = Vec::new();
    config.tools.configured.push(ConfiguredToolConfig {
        name: "mcp_hidden".to_owned(),
        description: "Hidden MCP command tool".to_owned(),
        input_schema: json!({ "type": "object", "properties": {} }),
        safety: ToolSafety::ReadOnly,
        timeout_ms: Some(1_000),
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Mcp {
            server: Some("hidden-mcp".to_owned()),
            command: "sh".to_owned(),
            args: vec!["-c".to_owned(), "exit 99".to_owned()],
            tool: "remote".to_owned(),
            protocol_version: "2025-06-18".to_owned(),
        },
    });
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
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
            &AgentTask {
                text: "mcp".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            ToolCall {
                id: new_call_id(),
                name: "mcp_hidden".to_owned(),
                args: json!({}),
            },
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

#[tokio::test]
async fn ask_write_hides_tools_that_need_unwired_approval_from_model() {
    let (output, _events) = run_with(test_config(), "summarize hello").await;

    assert!(output.contains("tools=3"));
}

#[tokio::test]
async fn plan_permission_mode_exposes_only_read_only_tools_even_when_interactive() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.permissions.mode = PermissionMode::Plan;
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::with_event_sink_and_approval_transport(
        config,
        dir.path().to_path_buf(),
        events,
        Arc::new(TestApprovalTransport { interactive: true }),
    )
    .unwrap();

    let output = runtime.run("summarize hello".to_owned()).await.unwrap();

    assert!(output.text.contains("tools=3"));
}

#[tokio::test]
async fn auto_permission_mode_exposes_non_dangerous_tools_without_approval_transport() {
    let mut config = test_config();
    config.permissions.mode = PermissionMode::Auto;

    let (output, _events) = run_with(config, "summarize hello").await;

    assert!(output.contains("tools=5"));
}

#[tokio::test]
async fn auto_permission_mode_hides_command_and_network_tools() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.permissions.mode = PermissionMode::Auto;
    let mut registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    registry.tools.register(NetworkTool).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Auto,
    );
    let orchestrator = ToolOrchestrator::default();

    let names = orchestrator
        .visible_tool_specs(&ctx, dir.path())
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();

    assert!(names.contains(&"read_file".to_owned()));
    assert!(names.contains(&"write_file".to_owned()));
    assert!(!names.contains(&"shell".to_owned()));
    assert!(!names.contains(&"network_probe".to_owned()));

    let denied = orchestrator
        .execute(
            &ctx,
            &AgentTask {
                text: "try shell".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            ToolCall {
                id: new_call_id(),
                name: "shell".to_owned(),
                args: json!({ "command": "echo should-not-run" }),
            },
        )
        .await
        .unwrap();

    assert!(!denied.ok);
    assert!(
        denied
            .error
            .as_deref()
            .is_some_and(|error| error.contains("permission mode auto denies"))
    );
    assert!(events.events().await.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result }
                if !result.ok
                    && result
                        .error
                        .as_deref()
                        .is_some_and(|error| error.contains("permission mode auto denies"))
        )
    }));
}

#[tokio::test]
async fn tool_orchestrator_enforces_tool_timeout() {
    let dir = temp_workspace();
    let config = test_config();
    let mut registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    registry.tools.register(SlowTool).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: false }),
        PermissionMode::Auto,
    );
    let orchestrator = ToolOrchestrator::default();

    let result = orchestrator
        .execute(
            &ctx,
            &AgentTask {
                text: "slow".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            ToolCall {
                id: new_call_id(),
                name: "slow".to_owned(),
                args: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert_eq!(result.metadata["timed_out"], true);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("tool timed out after 5ms"))
    );
    assert!(events.events().await.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result }
                if result.metadata["timed_out"] == true
        )
    }));
}

#[tokio::test]
async fn interactive_approval_transport_exposes_ask_tools_to_model() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::with_event_sink_and_approval_transport(
        test_config(),
        dir.path().to_path_buf(),
        events,
        Arc::new(TestApprovalTransport { interactive: true }),
    )
    .unwrap();

    let output = runtime.run("summarize hello".to_owned()).await.unwrap();

    assert!(output.text.contains("tools=6"));
}

#[tokio::test]
async fn malformed_tool_call_is_rejected_before_approval() {
    let dir = temp_workspace();
    let config = test_config();
    let registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
    );

    let result = ToolOrchestrator::default()
        .execute(
            &ctx,
            &AgentTask {
                text: "write malformed".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            ToolCall {
                id: new_call_id(),
                name: "write_file".to_owned(),
                args: json!({}),
            },
        )
        .await
        .unwrap();

    assert!(!result.ok);
    assert!(
        result.metadata["validation_error"]
            .as_bool()
            .unwrap_or(false)
    );
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("tool 'write_file' requires string arg 'path'"))
    );
    let records = events.events().await;
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result } if result.metadata["validation_error"] == true
        )
    }));
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalRequested { .. }))
    );
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalResolved { .. }))
    );
}

#[tokio::test]
async fn workflow_does_not_execute_tool_calls_from_length_response() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.policy = "allow_all".to_owned();
    let mut registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    registry.model = Arc::new(LengthToolCallModel);
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
    );

    let output = SingleLoopWorkflow::default()
        .run(
            AgentTask {
                text: "write".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            Vec::new(),
            ctx,
        )
        .await
        .unwrap();

    assert_eq!(output.output.text, "partial write");
    assert!(!dir.path().join("partial.txt").exists());
    let records = events.events().await;
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ToolCallRequested { .. }))
    );
    assert!(
        !records
            .iter()
            .any(|event| matches!(event, Event::ApprovalRequested { .. }))
    );
}

#[tokio::test]
async fn workflow_requests_final_answer_without_tools_after_round_limit() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.policy = "allow_all".to_owned();
    let mut registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    registry.model = Arc::new(FinalAfterToolLimitModel::default());
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        Arc::new(EventEmitter::new(events.clone())),
        Arc::new(TestApprovalTransport { interactive: true }),
        PermissionMode::Normal,
    );

    let output = SingleLoopWorkflow { max_tool_rounds: 1 }
        .run(
            AgentTask {
                text: "write then finish".to_owned(),
                cwd: dir.path().to_path_buf(),
            },
            Vec::new(),
            ctx,
        )
        .await
        .unwrap();

    assert_eq!(output.output.text, "final after tool limit");
    assert_eq!(
        output.output.metadata["tool_round_limit_reached"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("limit-final.txt")).unwrap(),
        "done"
    );
    let records = events.events().await;
    assert_eq!(
        records
            .iter()
            .filter(|event| matches!(event, Event::ToolCallRequested { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn allow_all_keeps_all_registered_tools_visible_to_model() {
    let mut config = test_config();
    config.modules.policy = "allow_all".to_owned();

    let (output, _events) = run_with(config, "summarize hello").await;

    assert!(output.contains("tools=6"));
}

#[derive(Debug)]
struct TestApprovalTransport {
    interactive: bool,
}

#[derive(Debug)]
struct NetworkTool;

#[derive(Debug)]
struct LengthToolCallModel;

#[derive(Debug, Default)]
struct FinalAfterToolLimitModel {
    calls: AtomicUsize,
}

#[async_trait]
impl Tool for NetworkTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "network_probe".to_owned(),
            description: "Synthetic network tool for policy tests".to_owned(),
            input_schema: json!({ "type": "object" }),
            safety: ToolSafety::Network,
            timeout_ms: Some(1_000),
            metadata: serde_json::Value::Null,
        }
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: true,
            output: "network".to_owned(),
            error: None,
            metadata: serde_json::Value::Null,
        })
    }
}

#[async_trait]
impl ModelClient for FinalAfterToolLimitModel {
    async fn complete(
        &self,
        request: CanonicalModelRequest,
    ) -> anyhow::Result<CanonicalModelResponse> {
        let call_number = self.calls.fetch_add(1, Ordering::SeqCst);
        if call_number == 0 {
            let call = ToolCall {
                id: new_call_id(),
                name: "write_file".to_owned(),
                args: json!({ "path": "limit-final.txt", "content": "done" }),
            };
            return Ok(CanonicalModelResponse {
                message: CanonicalMessage {
                    id: modular_agent::domain::new_message_id(),
                    role: MessageRole::Assistant,
                    parts: vec![ContentPart::ToolCall { call: call.clone() }],
                    name: None,
                    tool_call_id: None,
                    metadata: serde_json::Value::Null,
                },
                tool_calls: vec![call],
                finish_reason: FinishReason::ToolCalls,
                usage: None,
                provider_metadata: serde_json::Value::Null,
            });
        }

        assert!(request.tools.is_empty());
        assert_eq!(request.tool_choice, ToolChoice::None);
        Ok(CanonicalModelResponse {
            message: CanonicalMessage::text(MessageRole::Assistant, "final after tool limit"),
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: None,
            provider_metadata: serde_json::Value::Null,
        })
    }
}

#[async_trait]
impl ModelClient for LengthToolCallModel {
    async fn complete(
        &self,
        _request: CanonicalModelRequest,
    ) -> anyhow::Result<CanonicalModelResponse> {
        let call = ToolCall {
            id: new_call_id(),
            name: "write_file".to_owned(),
            args: json!({ "path": "partial.txt", "content": "should not write" }),
        };
        Ok(CanonicalModelResponse {
            message: CanonicalMessage {
                id: modular_agent::domain::new_message_id(),
                role: MessageRole::Assistant,
                parts: vec![
                    ContentPart::Text {
                        text: "partial write".to_owned(),
                    },
                    ContentPart::ToolCall { call: call.clone() },
                ],
                name: None,
                tool_call_id: None,
                metadata: serde_json::Value::Null,
            },
            tool_calls: vec![call],
            finish_reason: FinishReason::Length,
            usage: None,
            provider_metadata: serde_json::Value::Null,
        })
    }
}

#[derive(Debug)]
struct SlowTool;

#[async_trait]
impl Tool for SlowTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "slow".to_owned(),
            description: "Synthetic slow tool for timeout tests".to_owned(),
            input_schema: json!({ "type": "object" }),
            safety: ToolSafety::ReadOnly,
            timeout_ms: Some(5),
            metadata: serde_json::Value::Null,
        }
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> anyhow::Result<ToolResult> {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: true,
            output: "done".to_owned(),
            error: None,
            metadata: serde_json::Value::Null,
        })
    }
}

#[async_trait]
impl ApprovalTransport for TestApprovalTransport {
    fn can_request_approval(&self) -> bool {
        self.interactive
    }

    async fn request_approval(
        &self,
        _request: ApprovalRequest,
    ) -> anyhow::Result<ApprovalResponse> {
        Ok(ApprovalResponse {
            approved: false,
            note: Some("test approval denied".to_owned()),
        })
    }
}

#[tokio::test]
async fn folder_listing_question_uses_list_dir_context() {
    let (output, events) = run_with(test_config(), "привет что в папке ?").await;

    assert!(output.contains("file\tsample.txt"));
    assert!(events.events().await.iter().any(|event| {
        matches!(
            event,
            Event::ToolCallRequested { call } if call.name == "list_dir"
        )
    }));
}

#[tokio::test]
async fn context_chunks_are_not_persisted_to_runtime_history() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime =
        AgentRuntime::with_event_sink(test_config(), dir.path().to_path_buf(), events).unwrap();

    let listing = runtime
        .run("привет что в папке ?".to_owned())
        .await
        .unwrap();
    let follow_up = runtime
        .run("summarize after listing".to_owned())
        .await
        .unwrap();

    assert!(listing.text.contains("file\tsample.txt"));
    assert!(follow_up.text.contains("context_chunks=1"));
    assert!(!follow_up.text.contains("after directory listing"));
    assert_eq!(runtime.history_len().await, 4);
}

#[tokio::test]
async fn context_chunks_are_not_written_to_session_store() {
    let dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("config file");
    let runtime = AgentRuntime::new_with_config_path(
        test_config(),
        dir.path().to_path_buf(),
        Some(&config_path),
    )
    .unwrap();

    let output = runtime
        .run("привет что в папке ?".to_owned())
        .await
        .unwrap();
    let messages_path = runtime.session_dir().unwrap().join("messages.jsonl");
    let contents = std::fs::read_to_string(messages_path).expect("messages jsonl");
    let messages = contents
        .lines()
        .map(|line| serde_json::from_str::<CanonicalMessage>(line).expect("message"))
        .collect::<Vec<_>>();

    assert!(output.text.contains("file\tsample.txt"));
    assert_eq!(messages.len(), 2);
    assert!(!messages.iter().any(|message| {
        message
            .parts
            .iter()
            .any(|part| matches!(part, ContentPart::Context { .. }))
    }));
}

#[tokio::test]
async fn list_dir_lists_workspace_entries() {
    let dir = temp_workspace();
    std::fs::create_dir(dir.path().join("src")).expect("src dir");
    std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}\n").expect("main file");
    let tool = ListDirTool;
    let call = ToolCall {
        id: new_call_id(),
        name: "list_dir".to_owned(),
        args: json!({ "path": "." }),
    };

    let result = tool
        .invoke(
            &call,
            ToolContext {
                cwd: dir.path().to_path_buf(),
            },
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert!(result.output.contains("file\tsample.txt"));
    assert!(result.output.contains("dir\tsrc"));
}

#[tokio::test]
async fn list_dir_rejects_parent_traversal() {
    let dir = temp_workspace();
    let tool = ListDirTool;
    let call = ToolCall {
        id: new_call_id(),
        name: "list_dir".to_owned(),
        args: json!({ "path": ".." }),
    };

    let error = tool
        .invoke(
            &call,
            ToolContext {
                cwd: dir.path().to_path_buf(),
            },
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("escapes workspace"));
}

#[tokio::test]
async fn read_file_directory_error_points_to_list_dir() {
    let dir = temp_workspace();
    let tool = ReadFileTool;
    let call = ToolCall {
        id: new_call_id(),
        name: "read_file".to_owned(),
        args: json!({ "path": "." }),
    };

    let error = tool
        .invoke(
            &call,
            ToolContext {
                cwd: dir.path().to_path_buf(),
            },
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("use list_dir"));
}

#[tokio::test]
async fn apply_patch_replaces_exact_text_once() {
    let dir = temp_workspace();
    let tool = ApplyPatchTool::new(Arc::new(DirectPatchApplier::new(dir.path().to_path_buf())));
    let call = ToolCall {
        id: new_call_id(),
        name: "apply_patch".to_owned(),
        args: json!({
            "patch": "*** Begin Patch\n*** Update File: sample.txt\n@@\n-hello modular agent\n+patched modular agent\n*** End Patch",
        }),
    };

    let result = tool
        .invoke(
            &call,
            ToolContext {
                cwd: dir.path().to_path_buf(),
            },
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert!(result.output.contains("updated sample.txt"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("sample.txt")).unwrap(),
        "patched modular agent\n"
    );
}

#[tokio::test]
async fn apply_patch_adds_new_file_from_internal_format() {
    let dir = temp_workspace();
    let tool = ApplyPatchTool::new(Arc::new(DirectPatchApplier::new(dir.path().to_path_buf())));
    let call = ToolCall {
        id: new_call_id(),
        name: "apply_patch".to_owned(),
        args: json!({
            "patch": "*** Begin Patch\n*** Add File: nested/new.txt\n+hello\n+patch\n*** End Patch",
        }),
    };

    let result = tool
        .invoke(
            &call,
            ToolContext {
                cwd: dir.path().to_path_buf(),
            },
        )
        .await
        .unwrap();

    assert!(result.ok);
    assert!(result.output.contains("added nested/new.txt"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("nested").join("new.txt")).unwrap(),
        "hello\npatch\n"
    );
}

#[tokio::test]
async fn apply_patch_rejects_parent_traversal() {
    let dir = temp_workspace();
    let tool = ApplyPatchTool::new(Arc::new(DirectPatchApplier::new(dir.path().to_path_buf())));
    let call = ToolCall {
        id: new_call_id(),
        name: "apply_patch".to_owned(),
        args: json!({
            "patch": "*** Begin Patch\n*** Add File: ../outside.txt\n+outside\n*** End Patch",
        }),
    };

    let error = tool
        .invoke(
            &call,
            ToolContext {
                cwd: dir.path().to_path_buf(),
            },
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("escapes workspace"));
}

#[test]
fn ask_write_rejects_unknown_policy_tool_at_startup() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.policy.ask_write.allow = vec!["missing_tool".to_owned()];

    let error = match BuiltinRegistry::from_config(&config, dir.path().to_path_buf()) {
        Ok(_) => panic!("unknown policy tool should be rejected"),
        Err(error) => error,
    };

    let error = error.to_string();
    assert!(error.contains("policy.ask_write.allow references unknown tool \"missing_tool\""));
    assert!(error.contains("registered tools:"));
    assert!(error.contains("read_file"));
    assert!(error.contains("hint: enable the builtin tool"));
}

#[test]
fn ask_write_rejects_unknown_ask_before_tool_at_startup() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.policy.ask_write.ask_before = vec!["missing_tool".to_owned()];

    let error = match BuiltinRegistry::from_config(&config, dir.path().to_path_buf()) {
        Ok(_) => panic!("unknown ask_before policy tool should be rejected"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("policy.ask_write.ask_before references unknown tool \"missing_tool\"")
    );
}

#[tokio::test]
async fn tool_invocation_error_is_returned_as_failed_tool_result() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime =
        AgentRuntime::with_event_sink(test_config(), dir.path().to_path_buf(), events.clone())
            .unwrap();

    let output = runtime
        .run("read_file missing.txt".to_owned())
        .await
        .unwrap();
    let records = events.events().await;

    assert!(output.text.contains("failed to canonicalize"));
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result }
                if !result.ok
                    && result
                        .error
                        .as_deref()
                        .is_some_and(|error| error.contains("failed to canonicalize"))
        )
    }));
}

#[tokio::test]
async fn write_file_rejects_parent_traversal() {
    let dir = temp_workspace();
    let outside = dir.path().parent().unwrap().join("outside-write.txt");
    let tool = WriteFileTool;
    let call = ToolCall {
        id: new_call_id(),
        name: "write_file".to_owned(),
        args: json!({ "path": "../outside-write.txt", "content": "escaped" }),
    };

    let error = tool
        .invoke(
            &call,
            ToolContext {
                cwd: dir.path().to_path_buf(),
            },
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("escapes workspace"));
    assert!(!outside.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn write_file_rejects_symlink_escape() {
    let dir = temp_workspace();
    let outside_dir = tempfile::tempdir().expect("outside dir");
    let link = dir.path().join("outside-link");
    std::os::unix::fs::symlink(outside_dir.path(), &link).expect("symlink");
    let tool = WriteFileTool;
    let call = ToolCall {
        id: new_call_id(),
        name: "write_file".to_owned(),
        args: json!({ "path": "outside-link/escape.txt", "content": "escaped" }),
    };

    let error = tool
        .invoke(
            &call,
            ToolContext {
                cwd: dir.path().to_path_buf(),
            },
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("escapes workspace"));
    assert!(!outside_dir.path().join("escape.txt").exists());
}

#[tokio::test]
async fn fake_model_uses_canonical_contract() {
    let model = ModelService::new(Arc::new(FakeModelClient));
    let request = CanonicalModelRequest {
        model: ModelRef {
            provider: "fake".to_owned(),
            model: "fake-tool-model".to_owned(),
        },
        instructions: Vec::new(),
        messages: vec![CanonicalMessage::text(
            MessageRole::User,
            "read_file sample.txt",
        )],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        response_format: ResponseFormat::Text,
        sampling: SamplingConfig::default(),
        reasoning: ReasoningConfig::default(),
        limits: ModelLimits::default(),
        cache: CacheHints::default(),
        metadata: serde_json::Value::Null,
    };

    let response = model.complete(request).await.unwrap();

    assert_eq!(response.finish_reason, FinishReason::ToolCalls);
    assert_eq!(response.tool_calls[0].name, "read_file");
}

#[tokio::test]
async fn model_service_shapes_request_before_adapter_call() {
    let model = ModelService::new(Arc::new(NoToolsAdapter));
    let request = CanonicalModelRequest {
        model: ModelRef {
            provider: "test".to_owned(),
            model: "no-tools".to_owned(),
        },
        instructions: Vec::new(),
        messages: vec![CanonicalMessage::text(MessageRole::User, "hello")],
        tools: vec![ToolSpec {
            name: "read_file".to_owned(),
            description: "read file".to_owned(),
            input_schema: json!({ "type": "object" }),
            safety: ToolSafety::ReadOnly,
            timeout_ms: None,
            metadata: serde_json::Value::Null,
        }],
        tool_choice: ToolChoice::Auto,
        response_format: ResponseFormat::Text,
        sampling: SamplingConfig::default(),
        reasoning: ReasoningConfig {
            effort: Some("high".to_owned()),
            summary: true,
        },
        limits: ModelLimits {
            max_input_tokens: Some(10_000),
            max_output_tokens: Some(10_000),
        },
        cache: CacheHints {
            cache_instructions: true,
            cache_context: true,
        },
        metadata: serde_json::Value::Null,
    };

    let response = model.complete(request).await.unwrap();

    assert_eq!(response.provider_metadata["tool_count"], 0);
    assert_eq!(response.provider_metadata["tool_choice"], "None");
    assert_eq!(
        response.provider_metadata["reasoning"],
        serde_json::Value::Null
    );
    assert_eq!(response.provider_metadata["cache"], serde_json::Value::Null);
    assert_eq!(response.provider_metadata["max_output_tokens"], 128);
}

struct NoToolsAdapter;

#[async_trait]
impl ModelAdapter for NoToolsAdapter {
    fn id(&self) -> &'static str {
        "test.no_tools"
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities {
            supports_tools: false,
            supports_parallel_tool_calls: false,
            supports_streaming: false,
            supports_json_schema: false,
            supports_system_role: true,
            supports_developer_role: true,
            supports_cache_hints: false,
            supports_reasoning_config: false,
            supports_image_input: false,
            supports_file_input: false,
            max_input_tokens: Some(512),
            max_output_tokens: Some(128),
        }
    }

    async fn complete(
        &self,
        request: CanonicalModelRequest,
    ) -> anyhow::Result<CanonicalModelResponse> {
        Ok(CanonicalModelResponse {
            message: CanonicalMessage::text(MessageRole::Assistant, "ok"),
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: None,
            provider_metadata: json!({
                "tool_count": request.tools.len(),
                "tool_choice": format!("{:?}", request.tool_choice),
                "reasoning": request.reasoning.effort,
                "cache": if request.cache == CacheHints::default() {
                    serde_json::Value::Null
                } else {
                    json!(request.cache)
                },
                "max_output_tokens": request.limits.max_output_tokens,
            }),
        })
    }
}

#[tokio::test]
async fn json_config_file_can_select_anthropic_provider() {
    let config =
        modular_agent::core::AppConfig::load(Some(std::path::Path::new("config.example.json")))
            .await
            .unwrap();
    let model_config = config.active_model_config().unwrap();

    assert_eq!(config.active_provider.as_deref(), Some("anthropic"));
    assert_eq!(model_config.provider, "anthropic");
    assert_eq!(model_config.provider_config["api_key"], "sk-ant-...");
    assert_eq!(
        model_config.provider_config["base_url"],
        "https://api.anthropic.com"
    );
    assert_eq!(config.context.simple.max_search_results, 50);
    assert_eq!(config.tools.enabled, standard_tool_names());
    assert!(configured_tool_names(&config).is_empty());
}

#[tokio::test]
async fn toml_config_file_can_select_statusline_renderer() {
    let config =
        modular_agent::core::AppConfig::load(Some(std::path::Path::new("agent.example.toml")))
            .await
            .unwrap();

    assert_eq!(config.modules.renderer, "statusline");
    assert_eq!(
        config.renderer.statusline.components,
        ["model", "context", "session"]
    );
    assert_eq!(config.renderer.statusline.position, "bottom");
    assert_eq!(config.renderer.statusline.frame, "block");
    assert_eq!(config.tools.enabled, standard_tool_names());
    assert!(configured_tool_names(&config).is_empty());
}

#[tokio::test]
async fn config_directory_merges_sorted_config_files() {
    let dir = tempfile::tempdir().expect("config dir");
    std::fs::write(
        dir.path().join("01-model.toml"),
        r#"
active_provider = "local"

[providers.local]
provider = "openai_compatible"
model = "local-model"
base_url = "http://127.0.0.1:11434/v1"
"#,
    )
    .expect("model config");
    std::fs::write(
        dir.path().join("02-runtime.toml"),
        r#"
[modules]
renderer = "statusline"
search = "rg"

[tools]
enabled = ["read_file", "search"]

[search.rg]
max_results = 7

[renderer.statusline]
components = ["model", "context"]
ansi = false
"#,
    )
    .expect("runtime config");

    let config = modular_agent::core::AppConfig::load(Some(dir.path()))
        .await
        .unwrap();
    let model_config = config.active_model_config().unwrap();

    assert_eq!(model_config.provider, "openai_compatible");
    assert_eq!(model_config.model, "local-model");
    assert_eq!(
        model_config.provider_config["base_url"],
        "http://127.0.0.1:11434/v1"
    );
    assert_eq!(config.modules.renderer, "statusline");
    assert_eq!(config.modules.search, "rg");
    assert_eq!(config.search.rg.max_results, 7);
    assert_eq!(config.tools.enabled, ["read_file", "search"]);
    assert_eq!(config.renderer.statusline.components, ["model", "context"]);
    assert!(!config.renderer.statusline.ansi);
}

#[tokio::test]
async fn config_directory_loads_tools_from_config_root_tools_dir_by_default() {
    let root = tempfile::tempdir().expect("config root");
    let configs_dir = root.path().join("configs");
    let tools_dir = root.path().join("tools");
    std::fs::create_dir(&configs_dir).expect("configs dir");
    std::fs::create_dir(&tools_dir).expect("tools dir");
    std::fs::write(
        configs_dir.join("01-runtime.toml"),
        r#"
[tools]
enabled = []
"#,
    )
    .expect("runtime config");
    std::fs::write(
        tools_dir.join("read-file.toml"),
        r#"
name = "read_file"
description = "Configured read tool from config root"
safety = "ReadOnly"
timeout_ms = 1000

[executor]
kind = "native"
handler = "read_file"
"#,
    )
    .expect("tool manifest");

    let config = modular_agent::core::AppConfig::load(Some(&configs_dir))
        .await
        .unwrap();

    assert!(config.tools.enabled.is_empty());
    assert_eq!(configured_tool_names(&config), ["read_file"]);
}

#[tokio::test]
async fn json_config_can_switch_to_custom_provider_url() {
    let mut config =
        modular_agent::core::AppConfig::load(Some(std::path::Path::new("config.example.json")))
            .await
            .unwrap();
    config.active_provider = Some("local".to_owned());

    let model_config = config.active_model_config().unwrap();

    assert_eq!(model_config.provider, "openai_compatible");
    assert_eq!(
        model_config.provider_config["base_url"],
        "http://127.0.0.1:11434/v1"
    );
}

#[test]
fn workspace_path_is_encoded_as_folder_name() {
    let encoded = modular_agent::core::encode_workspace_path(std::path::Path::new("/home/game"));

    assert_eq!(encoded, "home|game");
}

#[test]
fn workspace_path_keeps_cyrillic_folder_names() {
    let encoded = modular_agent::core::encode_workspace_path(std::path::Path::new(
        "/home/qweasd123tg/Проекты/моя игра",
    ));

    assert_eq!(encoded, "home|qweasd123tg|Проекты|моя_игра");
}
