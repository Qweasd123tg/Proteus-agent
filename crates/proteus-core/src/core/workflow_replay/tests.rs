use std::collections::BTreeMap;

use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    plugin::{
        PluginApprovalPolicy_TO, PluginWorkflow, PluginWorkflow_TO, PluginWorkflowError,
        PluginWorkflowHostMut, PluginWorkflowInput, PluginWorkflowOutput,
    },
};
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::{
    core::{
        JournalEntry, ModelRequestRecorded, ModelResponseOutcome, ModelResponseRecorded,
        ModulesConfig, SessionConfigSnapshot, SessionConfigTool, SessionStore, ToolCallRecordPhase,
        ToolCallRecorded, ToolResultRecorded, TurnOpened, TurnSettled,
    },
    domain::{
        AgentOutput, AgentTask, ModelRef, PermissionMode, ReasoningConfig, ToolCall,
        ToolCallResolution, ToolResult, ToolSafety, ToolSpec, new_exchange_id, new_session_id,
        new_thread_id, new_turn_id,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole,
    },
};

mod compaction;
mod terminal;

const WORKFLOW_ID: &str = "replay.probe";
const POLICY_ID: &str = "replay.allow";

#[derive(Clone, Copy)]
struct ProbeWorkflow {
    diverge: bool,
}

impl PluginWorkflow for ProbeWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        match run_probe_workflow(self.diverge, input_json.as_str(), host) {
            Ok(output) => RResult::ROk(RString::from(
                serde_json::to_string(&output).expect("serialize probe output"),
            )),
            Err(error) => RResult::RErr(PluginWorkflowError::new(format!("{error:#}"))),
        }
    }
}

fn run_probe_workflow(
    diverge: bool,
    input_json: &str,
    host: &mut PluginWorkflowHostMut<'_>,
) -> anyhow::Result<PluginWorkflowOutput> {
    let input: PluginWorkflowInput = serde_json::from_str(input_json)?;
    let spec = probe_tool_spec();
    let mut first =
        CanonicalModelRequest::new(input.runtime.model_ref.clone(), input.history.clone())
            .with_tools(vec![spec.clone()]);
    if diverge {
        first.metadata = json!({ "implementation_changed": true });
    }
    let first_response = complete(host, &first)?;
    let mut new_messages = vec![first_response.message.clone()];
    let call = first_response
        .tool_calls
        .first()
        .ok_or_else(|| anyhow::anyhow!("probe response omitted tool call"))?;
    let result = execute(host, &input.task, call)?;
    let tool_message = CanonicalMessage::new(
        MessageRole::Tool,
        vec![ContentPart::ToolResult {
            result: result.clone(),
        }],
    )
    .with_tool_call_id(result.call_id.clone());
    new_messages.push(tool_message.clone());

    let mut second_messages = input.history;
    second_messages.extend(new_messages.iter().cloned());
    let second =
        CanonicalModelRequest::new(input.runtime.model_ref, second_messages).with_tools(vec![spec]);
    let second_response = complete(host, &second)?;
    new_messages.push(second_response.message);
    Ok(PluginWorkflowOutput {
        output: probe_output(&result),
        new_messages,
        history_replacement: None,
        compactions: Vec::new(),
    })
}

fn complete(
    host: &mut PluginWorkflowHostMut<'_>,
    request: &CanonicalModelRequest,
) -> anyhow::Result<CanonicalModelResponse> {
    let request = RString::from(serde_json::to_string(request)?);
    match host.complete_model_json(request) {
        RResult::ROk(response) => Ok(serde_json::from_str(response.as_str())?),
        RResult::RErr(error) => anyhow::bail!(error.message.into_string()),
    }
}

fn execute(
    host: &mut PluginWorkflowHostMut<'_>,
    task: &AgentTask,
    call: &ToolCall,
) -> anyhow::Result<ToolResult> {
    let task = RString::from(serde_json::to_string(task)?);
    let call = RString::from(serde_json::to_string(call)?);
    match host.execute_tool_json(task, call) {
        RResult::ROk(result) => Ok(serde_json::from_str(result.as_str())?),
        RResult::RErr(error) => anyhow::bail!(error.message.into_string()),
    }
}

struct TestJournal {
    _config_dir: TempDir,
    _workspace: TempDir,
    store: SessionStore,
    thread_id: crate::domain::ThreadId,
    turn_id: crate::domain::TurnId,
}

impl TestJournal {
    async fn new() -> Self {
        let config_dir = tempfile::tempdir().expect("config dir");
        let workspace = tempfile::tempdir().expect("workspace");
        let session_id = new_session_id();
        let thread_id = new_thread_id();
        let turn_id = new_turn_id();
        let store = SessionStore::new(config_dir.path(), workspace.path(), session_id)
            .expect("session store");
        let task = AgentTask::new("run replay probe", workspace.path().to_path_buf());
        let user = CanonicalMessage::text(MessageRole::User, task.text.clone());
        let spec = probe_tool_spec();
        let snapshot = snapshot(&spec);

        store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::TurnOpened(TurnOpened {
                    task: task.clone(),
                    base_history_revision: 0,
                    module_epoch: 3,
                    config_snapshot: serde_json::to_value(snapshot).expect("snapshot value"),
                }),
            )
            .await
            .expect("turn opened");
        store
            .append_history(thread_id, Some(turn_id), std::slice::from_ref(&user))
            .await
            .expect("user history");

        let call = ToolCall::new(
            "recorded-call",
            spec.name.clone(),
            json!({ "value": "safe" }),
        );
        let first_response = CanonicalModelResponse::new(
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            ),
            vec![call.clone()],
            FinishReason::ToolCalls,
        );
        let first_request = recorded_request(
            session_id,
            thread_id,
            turn_id,
            vec![user.clone()],
            spec.clone(),
        );
        append_exchange(
            &store,
            thread_id,
            turn_id,
            first_request,
            first_response.clone(),
        )
        .await;

        let result = ToolResult::ok(call.id.clone(), "recorded safe result").with_metadata(json!({
            "duration_ms": 42_424_242,
            "tool": spec.name,
        }));
        store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::ToolCallRecorded(ToolCallRecorded {
                    call: call.clone(),
                    phase: ToolCallRecordPhase::Requested,
                }),
            )
            .await
            .expect("tool requested");
        store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::ToolCallRecorded(ToolCallRecorded {
                    call: call.clone(),
                    phase: ToolCallRecordPhase::Resolved {
                        resolution: ToolCallResolution::Allowed,
                    },
                }),
            )
            .await
            .expect("tool resolved");
        store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::ToolResultRecorded(ToolResultRecorded {
                    result: result.clone(),
                }),
            )
            .await
            .expect("tool result");

        let source_tool_message = CanonicalMessage::new(
            MessageRole::Tool,
            vec![ContentPart::ToolResult {
                result: result.clone(),
            }],
        )
        .with_tool_call_id(result.call_id.clone());
        let second_response = CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "done"),
            Vec::new(),
            FinishReason::Stop,
        );
        let second_request = recorded_request(
            session_id,
            thread_id,
            turn_id,
            vec![
                user,
                first_response.message.clone(),
                source_tool_message.clone(),
            ],
            spec,
        );
        append_exchange(
            &store,
            thread_id,
            turn_id,
            second_request,
            second_response.clone(),
        )
        .await;

        let new_messages = vec![
            first_response.message,
            source_tool_message,
            second_response.message,
        ];
        store
            .append_history(thread_id, Some(turn_id), &new_messages)
            .await
            .expect("assistant history");
        store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::TurnSettled(TurnSettled {
                    status: TurnSettlementStatus::Success,
                    output: Some(probe_output(&result)),
                    error: None,
                }),
            )
            .await
            .expect("turn settled");

        Self {
            _config_dir: config_dir,
            _workspace: workspace,
            store,
            thread_id,
            turn_id,
        }
    }

    async fn append_second_turn(&self) -> crate::domain::TurnId {
        let turn_id = new_turn_id();
        let task = AgentTask::new("second recorded turn", self._workspace.path().to_path_buf());
        let user = CanonicalMessage::text(MessageRole::User, task.text.clone());
        let spec = probe_tool_spec();
        let revision = self
            .store
            .load_projection()
            .expect("projection")
            .history_revision;
        self.store
            .append_journal_entry(
                self.thread_id,
                Some(turn_id),
                JournalEntry::TurnOpened(TurnOpened {
                    task,
                    base_history_revision: revision,
                    module_epoch: 3,
                    config_snapshot: serde_json::to_value(snapshot(&spec)).expect("snapshot value"),
                }),
            )
            .await
            .expect("second turn opened");
        self.store
            .append_history(self.thread_id, Some(turn_id), std::slice::from_ref(&user))
            .await
            .expect("second user history");
        let response = CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "second"),
            Vec::new(),
            FinishReason::Stop,
        );
        append_exchange(
            &self.store,
            self.thread_id,
            turn_id,
            recorded_request(
                self.store.session_id(),
                self.thread_id,
                turn_id,
                vec![user],
                spec,
            ),
            response.clone(),
        )
        .await;
        self.store
            .append_history(
                self.thread_id,
                Some(turn_id),
                std::slice::from_ref(&response.message),
            )
            .await
            .expect("second assistant history");
        self.store
            .append_journal_entry(
                self.thread_id,
                Some(turn_id),
                JournalEntry::TurnSettled(TurnSettled {
                    status: TurnSettlementStatus::Success,
                    output: Some(AgentOutput::text("second")),
                    error: None,
                }),
            )
            .await
            .expect("second turn settled");
        turn_id
    }
}

async fn append_exchange(
    store: &SessionStore,
    thread_id: crate::domain::ThreadId,
    turn_id: crate::domain::TurnId,
    request: CanonicalModelRequest,
    response: CanonicalModelResponse,
) {
    let exchange_id = new_exchange_id();
    store
        .append_journal_entry(
            thread_id,
            Some(turn_id),
            JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                exchange_id,
                request,
            }),
        )
        .await
        .expect("model request");
    store
        .append_journal_entry(
            thread_id,
            Some(turn_id),
            JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                exchange_id,
                outcome: ModelResponseOutcome::Response { response },
            }),
        )
        .await
        .expect("model response");
}

fn recorded_request(
    session_id: crate::domain::SessionId,
    thread_id: crate::domain::ThreadId,
    turn_id: crate::domain::TurnId,
    messages: Vec<CanonicalMessage>,
    spec: ToolSpec,
) -> CanonicalModelRequest {
    let mut metadata = BTreeMap::new();
    metadata.insert("session_id".to_owned(), session_id.to_string());
    metadata.insert("thread_id".to_owned(), thread_id.to_string());
    metadata.insert("turn_id".to_owned(), turn_id.to_string());
    CanonicalModelRequest::new(ModelRef::new("missing-provider", "offline-model"), messages)
        .with_tools(vec![spec])
        .with_client_metadata(metadata)
}

fn snapshot(spec: &ToolSpec) -> SessionConfigSnapshot {
    let modules = ModulesConfig {
        workflow: WORKFLOW_ID.to_owned(),
        policy: POLICY_ID.to_owned(),
        ..ModulesConfig::default()
    };
    SessionConfigSnapshot {
        schema_version: 2,
        ts: 1,
        profile_name: "replay-test".to_owned(),
        active_provider: "missing-provider".to_owned(),
        model: ModelRef::new("missing-provider", "offline-model"),
        reasoning: ReasoningConfig::default(),
        modules,
        subagent_surface: "none".to_owned(),
        tools: vec![SessionConfigTool {
            source: "test".to_owned(),
            spec: spec.clone(),
        }],
        permission_mode_default: PermissionMode::Normal,
    }
}

fn probe_tool_spec() -> ToolSpec {
    ToolSpec::new(
        "write_marker",
        "A tool that must never execute during replay",
        json!({
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }),
        ToolSafety::WritesFiles,
    )
}

fn probe_output(result: &ToolResult) -> AgentOutput {
    AgentOutput::new(
        "done",
        json!({
            "probe": true,
            "context": {
                "token_estimate": result.metadata.to_string().len(),
            },
        }),
    )
}

fn catalog(diverge: bool) -> ModuleCatalog {
    let mut catalog = ModuleCatalog::new();
    let workflow = PluginWorkflow_TO::from_value(ProbeWorkflow { diverge }, TD_Opaque);
    catalog
        .register_plugin_workflow(WORKFLOW_ID, workflow)
        .expect("register workflow");
    let policy = PluginApprovalPolicy_TO::from_value(policy_pack::AllowAllPolicyPlugin, TD_Opaque);
    catalog
        .register_plugin_policy(POLICY_ID, policy)
        .expect("register policy");
    catalog
}

#[tokio::test]
async fn successful_turn_replays_model_and_tool_outcomes_without_side_effects() {
    let journal = TestJournal::new().await;
    let marker = journal._workspace.path().join("must-not-exist");
    let before = std::fs::read(journal.store.journal_path()).expect("journal before");

    let report = replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &catalog(false),
        WorkflowReplayOptions::default(),
    )
    .await
    .expect("workflow replay");

    let after = std::fs::read(journal.store.journal_path()).expect("journal after");
    assert!(report.comparison.matched, "{:?}", report.comparison.issues);
    assert_eq!(report.source.turn_id, journal.turn_id);
    assert_eq!(report.model_exchanges.recorded, 2);
    assert_eq!(report.model_exchanges.replayed, 2);
    assert_eq!(report.tool_calls.recorded, 1);
    assert_eq!(report.tool_calls.replayed, 1);
    assert_eq!(report.comparison.output_equal, Some(true));
    assert_eq!(report.comparison.history_equal, Some(true));
    assert_ne!(
        report
            .recorded
            .output
            .as_ref()
            .expect("recorded output")
            .metadata["context"]["token_estimate"],
        report
            .replay
            .output
            .as_ref()
            .expect("replay output")
            .metadata["context"]["token_estimate"]
    );
    assert_eq!(before, after);
    assert!(!marker.exists());
}

#[tokio::test]
async fn request_divergence_is_reported_and_stops_before_tool_replay() {
    let journal = TestJournal::new().await;

    let report = replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &catalog(true),
        WorkflowReplayOptions::default(),
    )
    .await
    .expect("divergence report");

    assert!(!report.comparison.matched);
    assert_eq!(report.model_exchanges.replayed, 0);
    assert_eq!(report.tool_calls.replayed, 0);
    assert!(
        report
            .comparison
            .issues
            .iter()
            .any(|issue| { issue.contains("model request #1") && issue.contains("metadata") })
    );
    assert!(report.source_journal_unchanged);
}

#[tokio::test]
async fn multiple_turns_require_an_explicit_turn_id() {
    let journal = TestJournal::new().await;
    let second_turn = journal.append_second_turn().await;

    let error = replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &catalog(false),
        WorkflowReplayOptions::default(),
    )
    .await
    .expect_err("ambiguous replay must fail");
    let message = error.to_string();
    assert!(message.contains("multiple turns"));
    assert!(message.contains(&journal.turn_id.to_string()));
    assert!(message.contains(&second_turn.to_string()));

    let report = replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &catalog(false),
        WorkflowReplayOptions {
            turn_id: Some(journal.turn_id),
        },
    )
    .await
    .expect("explicit first turn replay");
    assert!(report.comparison.matched, "{:?}", report.comparison.issues);
}
