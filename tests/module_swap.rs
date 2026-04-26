use std::sync::Arc;

use modular_agent::{
    contracts::{ModelClient, PolicyContext},
    core::{AgentRuntime, AppConfig, BuiltinRegistry, InMemoryEventStore},
    domain::{
        CacheHints, ModelLimits, ModelRef, PolicyDecision, ReasoningConfig, ResponseFormat,
        SamplingConfig, ToolCall, ToolChoice, new_call_id,
    },
    model_standard::{CanonicalMessage, CanonicalModelRequest, FinishReason, MessageRole},
    modules::FakeModelClient,
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
