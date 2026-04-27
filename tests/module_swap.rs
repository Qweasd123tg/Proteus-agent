use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use modular_agent::{
    contracts::{
        ApprovalRequest, ApprovalResponse, ApprovalTransport, ModelAdapter, ModelClient,
        PolicyContext, Tool, ToolContext, ToolRegistry, ToolSource,
    },
    core::{AgentRuntime, AppConfig, BuiltinRegistry, InMemoryEventStore, ToolOrchestrator},
    domain::{
        AgentTask, CacheHints, Event, ModelLimits, ModelRef, PermissionMode, PolicyDecision,
        ReasoningConfig, ResponseFormat, SamplingConfig, ToolCall, ToolChoice, ToolResult,
        ToolSafety, ToolSpec, new_call_id, new_session_id,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, FinishReason, MessageRole,
        ModelCapabilities,
    },
    modules::{
        ApplyPatchTool, DirectPatchApplier, FakeModelClient, ListDirTool, ModelService,
        ReadFileTool, WriteFileTool,
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

#[tokio::test]
async fn statusline_renderer_composes_configured_components() {
    let dir = temp_workspace();
    let mut config = AppConfig::default();
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
    let mut config = AppConfig::default();
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
    let mut config = AppConfig::default();
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
    let mut config = AppConfig::default();
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
async fn swapping_search_backend_does_not_change_runtime() {
    for search in ["null", "rg"] {
        let mut config = AppConfig::default();
        config.modules.search = search.to_owned();

        let (output, events) = run_with(config, "summarize hello").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn swapping_memory_store_does_not_change_runtime() {
    for memory in ["none", "jsonl"] {
        let mut config = AppConfig::default();
        config.modules.memory = memory.to_owned();

        let (output, events) = run_with(config, "summarize memory").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn swapping_policy_does_not_change_read_tool_execution() {
    for policy in ["allow_all", "ask_write"] {
        let mut config = AppConfig::default();
        config.modules.policy = policy.to_owned();

        let (output, events) = run_with(config, "read_file sample.txt").await;

        assert!(output.contains("hello modular agent"));
        assert!(events.events().await.len() >= 8);
    }
}

#[tokio::test]
async fn tool_visibility_and_execution_policy_are_separate() {
    let dir = temp_workspace();
    let config = AppConfig::default();
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
    let config = AppConfig::default();
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
async fn ask_write_hides_tools_that_need_unwired_approval_from_model() {
    let (output, _events) = run_with(AppConfig::default(), "summarize hello").await;

    assert!(output.contains("tools=3"));
}

#[tokio::test]
async fn plan_permission_mode_exposes_only_read_only_tools_even_when_interactive() {
    let dir = temp_workspace();
    let mut config = AppConfig::default();
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
    let mut config = AppConfig::default();
    config.permissions.mode = PermissionMode::Auto;

    let (output, _events) = run_with(config, "summarize hello").await;

    assert!(output.contains("tools=5"));
}

#[tokio::test]
async fn auto_permission_mode_hides_command_and_network_tools() {
    let dir = temp_workspace();
    let mut config = AppConfig::default();
    config.permissions.mode = PermissionMode::Auto;
    let mut registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    registry.tools.register(NetworkTool).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        events.clone(),
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
    let config = AppConfig::default();
    let mut registry = BuiltinRegistry::from_config(&config, dir.path().to_path_buf()).unwrap();
    registry.tools.register(SlowTool).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = registry.runtime_context(
        new_session_id(),
        events.clone(),
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
        AppConfig::default(),
        dir.path().to_path_buf(),
        events,
        Arc::new(TestApprovalTransport { interactive: true }),
    )
    .unwrap();

    let output = runtime.run("summarize hello".to_owned()).await.unwrap();

    assert!(output.text.contains("tools=6"));
}

#[tokio::test]
async fn allow_all_keeps_all_registered_tools_visible_to_model() {
    let mut config = AppConfig::default();
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
    let (output, events) = run_with(AppConfig::default(), "привет что в папке ?").await;

    assert!(output.contains("file\tsample.txt"));
    assert!(events.events().await.iter().any(|event| {
        matches!(
            event,
            Event::ToolCallRequested { call } if call.name == "list_dir"
        )
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
    let mut config = AppConfig::default();
    config.policy.ask_write.allow = vec!["missing_tool".to_owned()];

    let error = match BuiltinRegistry::from_config(&config, dir.path().to_path_buf()) {
        Ok(_) => panic!("unknown policy tool should be rejected"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("policy.ask_write.allow references unsupported tool: missing_tool")
    );
}

#[tokio::test]
async fn tool_invocation_error_is_returned_as_failed_tool_result() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::with_event_sink(
        AppConfig::default(),
        dir.path().to_path_buf(),
        events.clone(),
    )
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
