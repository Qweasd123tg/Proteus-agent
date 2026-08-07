use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use proteus_core::{
    contracts::{CompactionHost, CompactionInput, CompactionOutput, SearchQuery},
    core::{AgentRuntime, AppConfig, ModuleCatalog, RuntimeRegistry},
    domain::{AgentTask, ModelRef},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, MessageRole,
    },
    process_adapters::ProcessAdapterConfig,
};
use serde_json::json;

fn workspace_file(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn descriptor(slot: &str, module_id: &str, fixture: &str, args: &[&str]) -> ProcessAdapterConfig {
    let mut process_args = vec![workspace_file(fixture).display().to_string()];
    process_args.extend(args.iter().map(|value| (*value).to_owned()));
    serde_json::from_value(json!({
        "slot": slot,
        "module_id": module_id,
        "command": "sh",
        "args": process_args,
        "timeout_ms": 3_000,
        "handshake_timeout_ms": 3_000
    }))
    .expect("valid process descriptor")
}

fn search_config(module_id: &str, mode: &str) -> AppConfig {
    let mut config = AppConfig::default();
    config.modules.search = Some(module_id.to_owned());
    config.process_modules.push(descriptor(
        "search",
        module_id,
        "crates/proteus-core/tests/fixtures/process_search.sh",
        &[mode, module_id],
    ));
    config
}

fn compactor_config(module_id: &str, mode: &str, marker: Option<&Path>) -> AppConfig {
    let mut args = vec![mode, module_id];
    let marker_text;
    if let Some(marker) = marker {
        marker_text = marker.display().to_string();
        args.push(&marker_text);
    }

    let mut config = AppConfig::default();
    config.modules.compactor = Some(module_id.to_owned());
    config.process_modules.push(descriptor(
        "compactor",
        module_id,
        "crates/proteus-core/tests/fixtures/process_compactor.sh",
        &args,
    ));
    config
}

fn registry(config: &AppConfig, cwd: &Path) -> anyhow::Result<RuntimeRegistry> {
    RuntimeRegistry::from_config(config, cwd.to_path_buf())
}

#[tokio::test]
async fn search_slot_swaps_process_modules_without_changing_canonical_contract() {
    let workspace = tempfile::tempdir().expect("workspace");
    let absent = registry(&AppConfig::default(), workspace.path()).expect("absent search");
    assert!(
        absent
            .search
            .search(SearchQuery::new(
                "needle",
                workspace.path().to_path_buf(),
                5,
            ))
            .await
            .expect("structural fallback")
            .is_empty()
    );

    for module_id in ["fixture_a", "fixture_b"] {
        let selected = registry(&search_config(module_id, "static"), workspace.path())
            .expect("selected process search");
        let chunks = selected
            .search
            .search(SearchQuery::new(
                "needle",
                workspace.path().to_path_buf(),
                5,
            ))
            .await
            .expect("canonical search response");

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source, format!("process:{module_id}"));
        assert_eq!(chunks[0].content, format!("hit from {module_id}"));
        assert_eq!(chunks[0].metadata["fixture"], true);
    }
}

#[test]
fn selected_module_requires_an_exact_process_descriptor() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut config = AppConfig::default();
    config.modules.search = Some("missing".to_owned());

    let error = match registry(&config, workspace.path()) {
        Ok(_) => panic!("missing descriptor must fail"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("unsupported search module: missing")
    );
}

#[test]
fn duplicate_process_identity_is_rejected_before_runtime_build() {
    let mut config = search_config("fixture", "static");
    config.process_modules.push(descriptor(
        "search",
        "fixture",
        "crates/proteus-core/tests/fixtures/process_search.sh",
        &["static", "fixture"],
    ));

    let error = match ModuleCatalog::from_config(&config) {
        Ok(_) => panic!("duplicate descriptor must fail"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("duplicate process module descriptor: search/fixture")
    );
}

#[test]
fn handshake_mismatch_is_a_snapshot_build_error() {
    let workspace = tempfile::tempdir().expect("workspace");
    let error = match registry(&search_config("fixture", "mismatch"), workspace.path()) {
        Ok(_) => panic!("mismatched slot must fail"),
        Err(error) => error,
    };
    let message = format!("{error:#}");
    assert!(message.contains("handshake"), "{message}");
    assert!(message.contains("slot mismatch"), "{message}");
}

#[tokio::test]
async fn selected_process_failure_never_falls_back_to_absence() {
    let workspace = tempfile::tempdir().expect("workspace");
    let selected = registry(&search_config("fixture", "error"), workspace.path())
        .expect("selected process search");

    let error = selected
        .search
        .search(SearchQuery::new(
            "needle",
            workspace.path().to_path_buf(),
            5,
        ))
        .await
        .expect_err("module error must propagate");
    let message = format!("{error:#}");
    assert!(message.contains("returned an error"), "{message}");
    assert!(message.contains("fixture search failure"), "{message}");
}

#[tokio::test]
async fn invalid_slot_response_is_rejected_without_legacy_shape() {
    let workspace = tempfile::tempdir().expect("workspace");
    let selected = registry(&search_config("fixture", "invalid"), workspace.path())
        .expect("selected process search");

    let error = selected
        .search
        .search(SearchQuery::new(
            "needle",
            workspace.path().to_path_buf(),
            5,
        ))
        .await
        .expect_err("bare array must be rejected");
    assert!(
        error.to_string().contains("returned an invalid response"),
        "{error:#}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn process_handshake_does_not_block_the_async_runtime() {
    let workspace = tempfile::tempdir().expect("workspace");
    let started = Instant::now();
    let build = AgentRuntime::builder(
        search_config("fixture", "slow_initialize"),
        workspace.path().to_path_buf(),
    )
    .build_async();
    let heartbeat = async {
        tokio::time::sleep(Duration::from_millis(40)).await;
        started.elapsed()
    };
    let (runtime, elapsed) = tokio::join!(build, heartbeat);

    assert!(
        elapsed < Duration::from_millis(250),
        "blocked for {elapsed:?}"
    );
    drop(runtime.expect("runtime"));
}

#[derive(Default)]
struct NoModelCompactionHost;

#[async_trait]
impl CompactionHost for NoModelCompactionHost {
    async fn complete_model(
        &self,
        _request: CanonicalModelRequest,
    ) -> anyhow::Result<CanonicalModelResponse> {
        anyhow::bail!("fixture must not request model completion")
    }
}

fn compaction_input(cwd: &Path) -> CompactionInput {
    CompactionInput::new(
        AgentTask::new("current task", cwd.to_path_buf()),
        ModelRef::new("fake", "fake-model"),
        vec![
            CanonicalMessage::text(MessageRole::User, "old task"),
            CanonicalMessage::text(MessageRole::Assistant, "old answer"),
        ],
    )
    .with_reason("test")
}

async fn compact(registry: &RuntimeRegistry, cwd: &Path) -> anyhow::Result<CompactionOutput> {
    registry
        .compactor
        .compact(compaction_input(cwd), Arc::new(NoModelCompactionHost))
        .await
}

#[tokio::test]
async fn dead_process_is_restarted_lazily_for_the_same_selected_module() {
    let workspace = tempfile::tempdir().expect("workspace");
    let marker = workspace.path().join("exited-once");
    let selected = registry(
        &compactor_config("fixture", "exit_once", Some(&marker)),
        workspace.path(),
    )
    .expect("selected process compactor");

    compact(&selected, workspace.path())
        .await
        .expect_err("first process must exit");
    let output = compact(&selected, workspace.path())
        .await
        .expect("next request must use a fresh process");

    assert!(marker.exists());
    assert!(!output.changed);
    assert_eq!(output.metadata["fixture"], true);
}
