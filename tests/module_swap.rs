use std::sync::Arc;

use modular_agent::{
    contracts::{ModelClient, PolicyContext, Tool, ToolContext, ToolRegistry},
    core::{AgentRuntime, AppConfig, BuiltinRegistry, InMemoryEventStore},
    domain::{
        CacheHints, ModelLimits, ModelRef, PolicyDecision, ReasoningConfig, ResponseFormat,
        SamplingConfig, ToolCall, ToolChoice, new_call_id,
    },
    model_standard::{CanonicalMessage, CanonicalModelRequest, FinishReason, MessageRole},
    modules::{FakeModelClient, ReadFileTool, WriteFileTool},
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

    assert_eq!(names, ["read_file", "search", "shell", "write_file"]);
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
    let model = FakeModelClient;
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
