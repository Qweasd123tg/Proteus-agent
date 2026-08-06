use super::*;

fn fixture_path() -> std::path::PathBuf {
    workspace_root_file("crates/proteus-core/tests/fixtures/process_compactor.sh")
}

fn reference_path() -> std::path::PathBuf {
    workspace_root_file("examples/modules/compactor-process/compact.py")
}

fn process_compactor_config(
    command: &str,
    args: Vec<String>,
    module_id: &str,
    strategy: serde_json::Value,
) -> AppConfig {
    let mut config = test_config();
    config.modules.compactor = "process".to_owned();
    set_module_config(
        &mut config,
        "compactor",
        "process",
        json!({
            "module_id": module_id,
            "command": command,
            "args": args,
            "timeout_ms": 3000,
            "strategy": strategy,
        }),
    );
    config
}

fn fixture_config(mode: &str) -> AppConfig {
    process_compactor_config(
        "sh",
        vec![fixture_path().display().to_string(), mode.to_owned()],
        "fixture",
        serde_json::Value::Null,
    )
}

fn fixture_config_with_marker(mode: &str, marker: &std::path::Path) -> AppConfig {
    process_compactor_config(
        "sh",
        vec![
            fixture_path().display().to_string(),
            mode.to_owned(),
            marker.display().to_string(),
        ],
        "fixture",
        serde_json::Value::Null,
    )
}

fn build_registry(config: &AppConfig, cwd: &std::path::Path) -> anyhow::Result<RuntimeRegistry> {
    RuntimeRegistry::from_catalog(config, cwd.to_path_buf(), test_catalog())
}

#[derive(Default)]
struct NoopCompactionHost;

#[async_trait]
impl proteus_core::contracts::CompactionHost for NoopCompactionHost {
    async fn complete_model(
        &self,
        _request: CanonicalModelRequest,
    ) -> anyhow::Result<CanonicalModelResponse> {
        anyhow::bail!("process compactor must not call the model host")
    }
}

fn compaction_input(cwd: &std::path::Path) -> proteus_core::contracts::CompactionInput {
    proteus_core::contracts::CompactionInput::new(
        AgentTask::new("current task", cwd.to_path_buf()),
        ModelRef::new("fake", "fake-model"),
        vec![
            CanonicalMessage::text(MessageRole::User, "old task"),
            CanonicalMessage::text(MessageRole::Assistant, "old answer"),
            CanonicalMessage::text(MessageRole::User, "current task"),
        ],
    )
    .with_reason("test")
    .with_token_estimate(Some(100))
}

async fn compact(
    registry: &RuntimeRegistry,
    input: proteus_core::contracts::CompactionInput,
) -> anyhow::Result<proteus_core::contracts::CompactionOutput> {
    registry
        .compactor
        .compact(input, Arc::new(NoopCompactionHost))
        .await
}

#[tokio::test]
async fn compactor_slot_swaps_none_and_process_implementations() {
    let dir = temp_workspace();
    let input = compaction_input(dir.path());

    let none = registry_from_test_config(&test_config(), dir.path());
    let none_output = compact(&none, input.clone()).await.expect("none compactor");
    assert!(!none_output.changed);
    assert_eq!(none_output.messages, input.messages);

    let process = build_registry(&fixture_config("echo"), dir.path()).expect("process registry");
    let process_output = compact(&process, input.clone())
        .await
        .expect("process compactor");
    assert!(!process_output.changed);
    assert!(process_output.messages.is_empty());
    assert_eq!(process_output.metadata["fixture"], true);
}

#[test]
fn compactor_handshake_mismatch_is_a_snapshot_build_error() {
    let dir = temp_workspace();
    let error = match build_registry(&fixture_config("mismatch"), dir.path()) {
        Ok(_) => panic!("mismatched process module must not build"),
        Err(error) => error,
    };

    let message = format!("{error:#}");
    assert!(message.contains("handshake failed"), "{message}");
    assert!(message.contains("slot mismatch"), "{message}");
}

#[tokio::test]
async fn compactor_process_failure_is_returned_without_none_fallback() {
    let dir = temp_workspace();
    let registry = build_registry(&fixture_config("error"), dir.path()).expect("registry");

    let error = compact(&registry, compaction_input(dir.path()))
        .await
        .expect_err("process error must propagate");

    let message = format!("{error:#}");
    assert!(message.contains("request failed"), "{message}");
    assert!(message.contains("fixture compaction failure"), "{message}");
}

#[tokio::test]
async fn dead_compactor_restarts_lazily_after_failed_request() {
    let dir = temp_workspace();
    let marker = dir.path().join("compactor-exited-once");
    let registry = build_registry(
        &fixture_config_with_marker("exit_once", &marker),
        dir.path(),
    )
    .expect("registry");
    let input = compaction_input(dir.path());

    compact(&registry, input.clone())
        .await
        .expect_err("first child must exit");
    let output = compact(&registry, input.clone())
        .await
        .expect("fresh child must serve the next request");

    assert!(marker.exists());
    assert!(!output.changed);
    assert!(output.messages.is_empty());
}

#[tokio::test]
async fn dead_compactor_is_returned_without_none_fallback() {
    let dir = temp_workspace();
    let registry = build_registry(&fixture_config("exit"), dir.path()).expect("registry");

    let error = compact(&registry, compaction_input(dir.path()))
        .await
        .expect_err("dead child must fail selected slot");

    assert!(error.to_string().contains("request failed"), "{error:#}");
}

#[tokio::test]
async fn bare_compaction_output_is_rejected_as_invalid_slot_response() {
    let dir = temp_workspace();
    let registry = build_registry(&fixture_config("invalid"), dir.path()).expect("registry");

    let error = compact(&registry, compaction_input(dir.path()))
        .await
        .expect_err("bare output must fail");

    assert!(
        error.to_string().contains("returned invalid response"),
        "{error:#}"
    );
}

#[tokio::test]
async fn reference_python_compactor_preserves_context_and_recent_turn_suffix() {
    if !std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return;
    }
    let dir = temp_workspace();
    let config = process_compactor_config(
        "python3",
        vec![reference_path().display().to_string()],
        "python_suffix",
        json!({
            "trigger_messages": 4,
            "retain_user_turns": 2,
        }),
    );
    let registry = build_registry(&config, dir.path()).expect("reference process registry");
    let context = CanonicalMessage::new(
        MessageRole::User,
        vec![ContentPart::Context {
            chunk: ContextChunk::new("fixture", "fresh context"),
        }],
    )
    .with_name(proteus_core::domain::CONTEXT_MESSAGE_NAME);
    let old_user = CanonicalMessage::text(MessageRole::User, "old task");
    let recent_user = CanonicalMessage::text(MessageRole::User, "recent task");
    let current_user = CanonicalMessage::text(MessageRole::User, "current task");
    let mut input = compaction_input(dir.path());
    input.messages = vec![
        context.clone(),
        old_user.clone(),
        CanonicalMessage::text(MessageRole::Assistant, "old answer"),
        recent_user.clone(),
        CanonicalMessage::text(MessageRole::Assistant, "recent answer"),
        current_user.clone(),
    ];

    let output = compact(&registry, input)
        .await
        .expect("reference compaction");

    assert!(output.changed);
    assert_eq!(output.messages.first(), Some(&context));
    assert!(
        !output
            .messages
            .iter()
            .any(|message| message.id == old_user.id)
    );
    assert!(
        output
            .messages
            .iter()
            .any(|message| message.id == recent_user.id)
    );
    assert!(
        output
            .messages
            .iter()
            .any(|message| message.id == current_user.id)
    );
    assert_eq!(output.metadata["summary_source"], "deterministic_suffix");
}

#[tokio::test]
async fn reference_python_compactor_handles_retention_larger_than_available_turns() {
    if !std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return;
    }
    let dir = temp_workspace();
    let config = process_compactor_config(
        "python3",
        vec![reference_path().display().to_string()],
        "python_suffix",
        json!({
            "trigger_messages": 2,
            "retain_user_turns": 3,
        }),
    );
    let registry = build_registry(&config, dir.path()).expect("reference process registry");
    let mut input = compaction_input(dir.path());
    input.messages = vec![
        CanonicalMessage::text(MessageRole::User, "only task"),
        CanonicalMessage::text(MessageRole::Assistant, "first answer"),
        CanonicalMessage::text(MessageRole::Assistant, "second answer"),
    ];
    let expected = input.messages.clone();

    let output = compact(&registry, input)
        .await
        .expect("retention larger than available turns must stay valid");

    assert!(!output.changed);
    assert_eq!(output.messages, expected);
    assert_eq!(
        output.metadata["skipped_reason"],
        "suffix_would_not_reduce_history"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn compactor_async_runtime_build_keeps_tokio_worker_responsive() {
    let dir = temp_workspace();
    let started = std::time::Instant::now();

    let build = AgentRuntime::builder(fixture_config("slow_initialize"), dir.path().to_path_buf())
        .with_module_catalog(test_catalog())
        .build_async();
    let heartbeat = async {
        tokio::time::sleep(Duration::from_millis(40)).await;
        started.elapsed()
    };
    let (runtime, heartbeat_elapsed) = tokio::join!(build, heartbeat);

    assert!(
        heartbeat_elapsed < Duration::from_millis(250),
        "process handshake blocked the Tokio worker for {heartbeat_elapsed:?}"
    );
    drop(runtime.expect("runtime"));
}
