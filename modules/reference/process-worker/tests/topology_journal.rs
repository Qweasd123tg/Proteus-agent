use std::{
    collections::HashSet,
    future::pending,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use proteus_contracts::{
    contracts::{
        COMPACTOR_HOST_COMPLETE_MODEL_METHOD, CONTEXT_HOST_RECALL_MEMORY_METHOD,
        CONTEXT_HOST_SEARCH_METHOD, CancellationToken, UserInputRequest, UserInputResponse,
        UserInputTransport, WORKFLOW_HOST_COMPLETE_MODEL_METHOD, WORKFLOW_HOST_EXECUTE_TOOL_METHOD,
    },
    domain::MemoryItem,
    model_standard::ContentPart,
};
use proteus_core::{
    core::{
        AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, ToolCallRecordPhase,
        TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
    },
    process_adapters::ProcessComponentConfig,
};
use proteus_module_protocol::current_process_contract_authority;
use serde_json::json;
use tokio::sync::Notify;

const COMPONENT_ID: &str = "reference-agent";

fn workspace_file(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(path)
}

fn worker_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_proteus-reference-worker"))
}

fn install_pid_recording_command(config: &mut AppConfig, marker: &Path) {
    let component = config
        .components
        .get(COMPONENT_ID)
        .expect("one-component profile")
        .clone();
    let mut value = serde_json::to_value(component).expect("component value");
    value["command"] = json!("/bin/sh");
    value["args"] = json!([
        "-c",
        "printf '%s\\n' \"$$\" >> \"$1\"\nexec \"$2\"",
        "p4-reference-agent",
        marker,
        worker_binary(),
    ]);
    config.components.insert(
        COMPONENT_ID.to_owned(),
        serde_json::from_value::<ProcessComponentConfig>(value).expect("instrumented component"),
    );
}

fn recorded_pids(marker: &Path) -> Vec<u32> {
    std::fs::read_to_string(marker)
        .expect("pid marker")
        .lines()
        .map(|line| line.parse::<u32>().expect("numeric worker pid"))
        .collect()
}

fn assert_separate_slot_authority(config: &AppConfig) {
    let component = config
        .components
        .get(COMPONENT_ID)
        .expect("one-component profile");
    let slots = component
        .exports()
        .map(|(slot, _, _)| slot)
        .collect::<HashSet<_>>();
    assert_eq!(
        slots,
        HashSet::from([
            "workflow",
            "search",
            "memory",
            "context",
            "policy",
            "patch",
            "compactor",
            "tool_exposure",
            "tool",
        ])
    );

    let workflow = current_process_contract_authority("workflow").expect("workflow authority");
    let context = current_process_contract_authority("context").expect("context authority");
    let compactor = current_process_contract_authority("compactor").expect("compactor authority");
    let tool = current_process_contract_authority("tool").expect("tool authority");

    assert!(workflow.allows_host_method(WORKFLOW_HOST_COMPLETE_MODEL_METHOD));
    assert!(workflow.allows_host_method(WORKFLOW_HOST_EXECUTE_TOOL_METHOD));
    assert!(!workflow.allows_host_method(CONTEXT_HOST_SEARCH_METHOD));
    assert!(context.allows_host_method(CONTEXT_HOST_SEARCH_METHOD));
    assert!(context.allows_host_method(CONTEXT_HOST_RECALL_MEMORY_METHOD));
    assert!(!context.allows_host_method(WORKFLOW_HOST_COMPLETE_MODEL_METHOD));
    assert!(compactor.allows_host_method(COMPACTOR_HOST_COMPLETE_MODEL_METHOD));
    assert!(!compactor.allows_host_method(WORKFLOW_HOST_EXECUTE_TOOL_METHOD));
    assert!(tool.host_methods.is_empty());
}

struct BlockingUserInput {
    started: Arc<Notify>,
}

#[async_trait]
impl UserInputTransport for BlockingUserInput {
    fn can_request_user_input(&self) -> bool {
        true
    }

    async fn request_user_input(
        &self,
        _request: UserInputRequest,
    ) -> anyhow::Result<UserInputResponse> {
        self.started.notify_one();
        pending::<anyhow::Result<UserInputResponse>>().await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_component_profile_preserves_pid_authority_cancellation_and_journal_replay() {
    let config_root = tempfile::tempdir().expect("config root");
    let workspace = tempfile::tempdir().expect("workspace");
    let marker = config_root.path().join("component-pids.txt");
    let config_path = config_root.path().join("config.toml");
    let profile_path = workspace_file("examples/configs/proteus.one-component.example.toml");
    let mut config = AppConfig::load(Some(&profile_path))
        .await
        .expect("one-component profile loads");
    install_pid_recording_command(&mut config, &marker);
    assert_separate_slot_authority(&config);
    std::fs::write(
        workspace.path().join("probe.txt"),
        "P4 tool call stayed in the shared component.\n",
    )
    .expect("probe file");

    let user_input_started = Arc::new(Notify::new());
    let runtime = Arc::new(
        AgentRuntime::builder(config.clone(), workspace.path().to_path_buf())
            .with_config_path(Some(&config_path))
            .with_module_catalog(ModuleCatalog::from_config(&config).expect("module catalog"))
            .with_user_input(Arc::new(BlockingUserInput {
                started: Arc::clone(&user_input_started),
            }))
            .build_async()
            .await
            .expect("one-component runtime"),
    );
    let initial_pids = recorded_pids(&marker);
    assert_eq!(
        initial_pids.len(),
        1,
        "catalog bootstrap starts one process"
    );
    let live_pid = initial_pids[0];

    let cancellation = CancellationToken::new();
    let canceled_run = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        let cancellation = cancellation.clone();
        async move {
            runtime
                .run_with_cancellation("request_user_input".to_owned(), cancellation)
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), user_input_started.notified())
        .await
        .expect("workflow reached blocking user-input callback");

    tokio::time::timeout(
        Duration::from_secs(5),
        runtime.remember(
            MemoryItem::new("fact", "independent sibling", json!({})),
            CancellationToken::new(),
        ),
    )
    .await
    .expect("independent memory invocation did not deadlock")
    .expect("independent memory invocation");
    assert_eq!(recorded_pids(&marker), [live_pid]);

    cancellation.cancel();
    let canceled_error = tokio::time::timeout(Duration::from_secs(5), canceled_run)
        .await
        .expect("canceled workflow settled")
        .expect("canceled workflow task")
        .expect_err("first turn must be canceled");
    assert!(
        format!("{canceled_error:#}").contains("canceled"),
        "{canceled_error:#}"
    );
    runtime
        .remember(
            MemoryItem::new("fact", "after cancellation", json!({})),
            CancellationToken::new(),
        )
        .await
        .expect("component remains usable after targeted cancellation");

    let output = runtime
        .run("read_file probe.txt".to_owned())
        .await
        .expect("complete process-backed workflow turn");
    assert!(
        output
            .text
            .contains("P4 tool call stayed in the shared component")
    );
    assert_eq!(
        recorded_pids(&marker),
        [live_pid],
        "cancel, sibling and full turn must keep one generation/PID"
    );

    let session_dir = runtime.session_dir().expect("session dir").to_path_buf();
    let store = SessionStore::open(session_dir.clone()).expect("session store");
    let projection = store.load_projection().expect("canonical projection");
    assert!(projection.unsettled_turns.is_empty());
    assert!(projection.interrupted_model_exchanges.is_empty());

    let settlements = projection
        .records
        .iter()
        .filter_map(|record| match (&record.turn_id, &record.entry) {
            (Some(turn_id), JournalEntry::TurnSettled(settled)) => Some((*turn_id, settled.status)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(settlements.len(), 2);
    assert_eq!(settlements[0].1, TurnSettlementStatus::Canceled);
    assert_eq!(settlements[1].1, TurnSettlementStatus::Success);
    let canceled_turn = settlements[0].0;
    let successful_turn = settlements[1].0;

    let canceled_calls = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::ToolCallRecorded(tool)
                if record.turn_id == Some(canceled_turn)
                    && tool.phase == ToolCallRecordPhase::Requested =>
            {
                Some(tool.call.id.clone())
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    assert!(
        projection
            .unresolved_tool_calls
            .iter()
            .all(|call_id| canceled_calls.contains(call_id)),
        "only externally canceled work may remain without a tool result"
    );

    let successful_tool_call = projection
        .records
        .iter()
        .find_map(|record| match &record.entry {
            JournalEntry::ToolCallRecorded(tool)
                if record.turn_id == Some(successful_turn)
                    && tool.phase == ToolCallRecordPhase::Requested
                    && tool.call.name == "read_file" =>
            {
                Some(tool.call.id.clone())
            }
            _ => None,
        });
    let successful_tool_call = successful_tool_call.expect("recorded process tool call");
    assert!(
        !projection
            .unresolved_tool_calls
            .contains(&successful_tool_call)
    );
    assert!(projection.records.iter().any(|record| {
        matches!(
            &record.entry,
            JournalEntry::ToolResultRecorded(result)
                if record.turn_id == Some(successful_turn)
                    && result.result.call_id == successful_tool_call
        )
    }));
    assert!(projection.records.iter().any(|record| {
        matches!(
            &record.entry,
            JournalEntry::ModelRequestRecorded(model)
                if record.turn_id == Some(successful_turn)
                    && model.request.messages.iter().any(|message| message.parts.iter().any(
                        |part| matches!(part.payload, ContentPart::Context { .. })
                    ))
        )
    }));

    drop(store);
    drop(runtime);

    let replay_catalog = ModuleCatalog::from_config(&config).expect("replay catalog");
    let replay = replay_workflow(
        &session_dir,
        &config,
        &replay_catalog,
        WorkflowReplayOptions {
            turn_id: Some(successful_turn),
        },
    )
    .await
    .expect("workflow replay");
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
    assert_eq!(replay.recorded.status, TurnSettlementStatus::Success);
    assert_eq!(replay.replay.status, TurnSettlementStatus::Success);
    assert_eq!(replay.tool_calls.recorded, 1);
    assert_eq!(replay.tool_calls.replayed, 1);
    drop(replay_catalog);

    let all_pids = recorded_pids(&marker);
    assert_eq!(
        all_pids.len(),
        2,
        "live run and replay each start one process"
    );
    assert_ne!(all_pids[0], all_pids[1]);
}
