use std::{path::Path, sync::Arc};

use proteus_core::{
    contracts::{CancellationToken, ExecutionScope, SearchQuery},
    core::{AppConfig, HeadlessApprovalTransport, ModuleEpoch, PreparedAssembly, RuntimeSnapshot},
    domain::PermissionMode,
    process_adapters::ProcessComponentConfig,
};
use serde_json::json;

fn workspace_file(path: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
        .display()
        .to_string()
}

fn process_search_config() -> AppConfig {
    let component: ProcessComponentConfig = serde_json::from_value(json!({
        "command": "sh",
        "args": [
            workspace_file("crates/proteus-core/tests/fixtures/process_search.sh"),
            "static",
            "execution-boundary-search",
            "execution-boundary-component",
        ],
        "handshake_timeout_ms": 3_000,
        "exports": {
            "search": {"execution-boundary-search": {"timeout_ms": 3_000}},
        },
    }))
    .expect("valid process component");
    let mut config = AppConfig::default();
    config.modules.search = Some("execution-boundary-search".to_owned());
    config
        .components
        .insert("execution-boundary-component".to_owned(), component);
    config
}

#[tokio::test]
async fn process_search_runs_through_execution_context_without_chat_identity() {
    let workspace = tempfile::tempdir().expect("workspace");
    let assembly = PreparedAssembly::from_config(
        process_search_config(),
        workspace.path().to_path_buf(),
        None,
    )
    .expect("prepared assembly");
    let snapshot = RuntimeSnapshot::new(ModuleEpoch::initial(), assembly, None);
    let execution = snapshot.registry.execution_context(
        ExecutionScope::fresh(CancellationToken::new()),
        Arc::new(HeadlessApprovalTransport),
        PermissionMode::Normal,
    );

    let chunks = execution
        .search
        .search(SearchQuery::new(
            "needle",
            workspace.path().to_path_buf(),
            5,
        ))
        .await
        .expect("process-backed search through generic execution boundary");

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].source, "process:execution-boundary-search");
    assert_eq!(chunks[0].content, "hit from execution-boundary-search");
    assert!(!execution.is_cancelled());
}
