use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use proteus_contracts::contracts::{ApprovalRequest, ApprovalResponse, ApprovalTransport};
use proteus_contracts::model_standard::MessageRole;
use proteus_core::{
    core::{
        AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, ToolCallRecordPhase,
        TurnSettlementStatus, WorkflowReplayOptions, read_eval_report, replay_workflow,
    },
    process_adapters::ProcessComponentConfig,
};
use serde_json::json;

struct ApprovingTransport;

#[async_trait]
impl ApprovalTransport for ApprovingTransport {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(
        &self,
        _request: ApprovalRequest,
    ) -> anyhow::Result<ApprovalResponse> {
        Ok(ApprovalResponse::approve())
    }
}

fn workspace_file(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(path)
}

fn component(value: serde_json::Value) -> ProcessComponentConfig {
    serde_json::from_value(value).expect("valid process component config")
}

async fn project_check_config() -> AppConfig {
    let profile = workspace_file("examples/configs/proteus.project-check.example.toml");
    let mut config = AppConfig::load(Some(&profile))
        .await
        .expect("project-check example profile");
    config.components.clear();
    config.components.insert(
        "project-check-controller".to_owned(),
        component(json!({
            "command": env!("CARGO_BIN_EXE_proteus-reference-worker"),
            "exports": {
                "workflow": { "coding.project_check": {} },
                "policy": { "ask_write": {} },
            },
        })),
    );
    config.components.insert(
        "project-check-tools".to_owned(),
        component(json!({
            "command": "python3",
            "args": [workspace_file(
                "modules/reference/process-worker/tests/fixtures/project_check_tools.py"
            )],
            "env": { "PYTHONDONTWRITEBYTECODE": "1" },
            "exports": {
                "tool": { "project-check-fixture-tools": {} },
            },
        })),
    );
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deterministic_controller_records_zero_model_calls_and_localizes_replay_gap() {
    let config_root = tempfile::tempdir().expect("config root");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = config_root.path().join("config.toml");
    let config = project_check_config().await;
    let catalog = ModuleCatalog::from_config(&config).expect("project-check module catalog");
    let runtime = AgentRuntime::builder(config.clone(), workspace.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(catalog)
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .expect("project-check runtime");

    let output = runtime
        .run("проверь проект".to_owned())
        .await
        .expect("deterministic project check");
    assert!(output.text.contains("завершена успешно"));
    assert_eq!(
        output.metadata["workflow"]["module_id"],
        "coding.project_check"
    );
    assert_eq!(output.metadata["project_check"]["status"], "passed");
    assert_eq!(output.metadata["project_check"]["model_calls"], 0);

    let session_dir = runtime.session_dir().expect("session dir").to_path_buf();
    let projection = SessionStore::open(session_dir.clone())
        .expect("session store")
        .load_projection()
        .expect("canonical projection");
    assert!(projection.unsettled_turns.is_empty());
    assert!(projection.interrupted_model_exchanges.is_empty());
    assert_eq!(projection.history.len(), 2);
    assert_eq!(projection.history[0].role, MessageRole::User);
    assert_eq!(projection.history[1].role, MessageRole::Assistant);

    let model_requests = projection
        .records
        .iter()
        .filter(|record| matches!(record.entry, JournalEntry::ModelRequestRecorded(_)))
        .count();
    assert_eq!(model_requests, 0);
    let requested_tools = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::ToolCallRecorded(tool)
                if tool.phase == ToolCallRecordPhase::Requested =>
            {
                Some(tool.call.name.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(requested_tools, ["git_status", "list_dir", "shell"]);
    let settlement = projection
        .records
        .iter()
        .find_map(|record| match &record.entry {
            JournalEntry::TurnSettled(settled) => Some(settled),
            _ => None,
        })
        .expect("turn settlement");
    assert_eq!(settlement.status, TurnSettlementStatus::Success);
    assert_eq!(
        settlement.output.as_ref().expect("settled output").metadata["project_check"]["status"],
        "passed"
    );
    let eval = read_eval_report(&session_dir).expect("model-free eval report");
    assert!(eval.succeeded());
    assert_eq!(eval.model_calls, 0);
    assert_eq!(eval.tool_calls, 3);
    assert_eq!(eval.tool_failures, 0);
    assert_eq!(eval.approvals_requested, 1);
    assert_eq!(eval.approvals_resolved, 1);
    assert_eq!(eval.approvals_approved, 1);

    drop(runtime);
    let replay_catalog = ModuleCatalog::from_config(&config).expect("replay module catalog");
    let replay_error = replay_workflow(
        &session_dir,
        &config,
        &replay_catalog,
        WorkflowReplayOptions::default(),
    )
    .await
    .expect_err("current workflow replay requires a model exchange");
    assert!(
        replay_error
            .to_string()
            .contains("contains no completed root model exchanges"),
        "{replay_error:#}"
    );
}
