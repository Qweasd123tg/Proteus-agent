use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use async_trait::async_trait;
use coding_workflow::CodingPlanExecuteReviewWorkflow;
use context_pack::SimpleContextBuilderPlugin;
use proteus_contracts::{
    abi_stable::sabi_trait::TD_Opaque,
    plugin::{PluginContextBuilder_TO, PluginWorkflow_TO},
};
use tokio::time::Duration;

use super::turn::{TurnAbort, turn_settlement_status};
use super::*;
use crate::{
    contracts::{RuntimeContext, Workflow, WorkflowOutput},
    core::{BuiltinModuleCatalog, ConfiguredToolConfig, ConfiguredToolExecutorConfig},
    domain::{AgentOutput, AgentTask, HistoryCompactionReport, ToolSafety},
    model_standard::{CanonicalMessage, CanonicalModelRequest, MessageRole},
};

mod steering_integration;

fn test_catalog() -> BuiltinModuleCatalog {
    let mut catalog = BuiltinModuleCatalog::new();
    catalog
        .register_plugin_context_builder(
            "simple",
            PluginContextBuilder_TO::from_value(SimpleContextBuilderPlugin, TD_Opaque),
        )
        .expect("register test context builder");
    catalog
        .register_plugin_workflow(
            "coding.plan_execute_review",
            PluginWorkflow_TO::from_value(CodingPlanExecuteReviewWorkflow, TD_Opaque),
        )
        .expect("register test workflow");
    catalog
}

fn message_text_for_test(message: &CanonicalMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match &part.payload {
            crate::model_standard::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn settlement_status_uses_typed_runtime_causes_not_error_text() {
    let misleading = anyhow::anyhow!("provider rejected field named cancellation_timeout");
    assert_eq!(
        turn_settlement_status(&misleading, false),
        crate::core::TurnSettlementStatus::Error
    );

    let timeout = anyhow::Error::new(TurnAbort::WorkflowTimeout { timeout_ms: 10 });
    assert_eq!(
        turn_settlement_status(&timeout, true),
        crate::core::TurnSettlementStatus::Timeout
    );

    assert_eq!(
        turn_settlement_status(&misleading, true),
        crate::core::TurnSettlementStatus::Canceled
    );
}

fn successful_messages(
    history: Vec<CanonicalMessage>,
    task: AgentTask,
    answer: impl Into<String>,
) -> WorkflowOutput {
    let current_user = history.last().expect("persisted current user message");
    assert_eq!(current_user.role, MessageRole::User);
    assert_eq!(message_text_for_test(current_user), task.text);
    let assistant = CanonicalMessage::text(MessageRole::Assistant, answer.into());
    WorkflowOutput::new(AgentOutput::text("done"), vec![assistant])
}

struct ShortHistoryWorkflow;
struct CompactingWorkflow;
struct HangingWorkflow;
struct DelayedWorkflow;
struct ModelCallingWorkflow;
struct SnapshotProbeWorkflow {
    wait_once: Arc<AtomicBool>,
    started: Arc<tokio::sync::Notify>,
    proceed: Arc<tokio::sync::Notify>,
}
async fn replace_workflow_for_test(runtime: &AgentRuntime, workflow: Arc<dyn Workflow>) {
    let mut snapshot = runtime.services.snapshot.write().await;
    snapshot.registry.workflow = workflow;
}

#[async_trait]
impl Workflow for ShortHistoryWorkflow {
    async fn run(
        &self,
        _task: AgentTask,
        _history: Vec<CanonicalMessage>,
        _ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        Ok(WorkflowOutput::new(
            AgentOutput::text("bad workflow"),
            Vec::new(),
        ))
    }
}

#[async_trait]
impl Workflow for CompactingWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        _ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        assert_eq!(history.len(), 3);
        let summary = CanonicalMessage::text(MessageRole::User, "compacted summary");
        let current = history.last().expect("current user").clone();
        assert_eq!(message_text_for_test(&current), task.text);
        let answer = CanonicalMessage::text(MessageRole::Assistant, "done after compact");
        let mut report =
            HistoryCompactionReport::unchanged(history.len(), Some("test_compaction".to_owned()));
        report.changed = true;
        report.output_messages = 3;
        report.original_token_estimate = Some(500);
        report.output_token_estimate = Some(50);
        report.trigger_tokens = Some(100);
        report.summary_source = Some("test".to_owned());
        report.summary = Some("compacted summary".to_owned());
        report.metadata = serde_json::json!({"test": true});
        Ok(WorkflowOutput::new(AgentOutput::text("done"), vec![answer])
            .with_history_replacement(vec![summary, current])
            .with_compactions(vec![report]))
    }
}

#[async_trait]
impl Workflow for HangingWorkflow {
    async fn run(
        &self,
        _task: AgentTask,
        _history: Vec<CanonicalMessage>,
        _ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(WorkflowOutput::new(
            AgentOutput::text("too late"),
            Vec::new(),
        ))
    }
}

#[async_trait]
impl Workflow for DelayedWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        _ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(successful_messages(history, task, "done"))
    }
}

#[async_trait]
impl Workflow for ModelCallingWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        let current_user = history.last().expect("persisted current user message");
        assert_eq!(message_text_for_test(current_user), task.text);
        let request = CanonicalModelRequest::new(ctx.model_ref.clone(), history)
            .with_instructions(ctx.instructions.clone())
            .with_tools(ctx.tools.specs())
            .with_reasoning(ctx.reasoning.clone());
        let response = ctx.model.complete(request).await?;
        Ok(WorkflowOutput::new(
            AgentOutput::text("done"),
            vec![response.message],
        ))
    }
}

#[async_trait]
impl Workflow for SnapshotProbeWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        if self.wait_once.swap(false, Ordering::SeqCst) {
            self.started.notify_one();
            self.proceed.notified().await;
        }
        let has_late_tool = ctx.tools.spec("late_tool").is_ok();
        let output = AgentOutput::text(format!("has_late_tool={has_late_tool}"));
        let current_user = history.last().expect("persisted current user message");
        assert_eq!(message_text_for_test(current_user), task.text);
        let assistant = CanonicalMessage::text(MessageRole::Assistant, output.text.clone());
        Ok(WorkflowOutput::new(output, vec![assistant]))
    }
}

#[tokio::test]
async fn run_errors_when_workflow_returns_no_turn_messages() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let mut config = AppConfig::default();
    config.modules.patch = "null".to_owned();
    let runtime = AgentRuntime::builder(config, cwd.path().to_path_buf())
        .with_module_catalog(test_catalog())
        .build()
        .expect("runtime");

    replace_workflow_for_test(&runtime, Arc::new(ShortHistoryWorkflow)).await;
    runtime
        .session
        .history
        .lock()
        .await
        .push(CanonicalMessage::text(MessageRole::User, "previous"));

    let error = runtime
        .run("current".to_owned())
        .await
        .expect_err("empty workflow turn must error");

    assert!(
        error
            .to_string()
            .contains("workflow returned no new persistent turn messages")
    );
}

#[tokio::test]
async fn failed_turn_keeps_user_message_in_runtime_and_session_store() {
    let config_root = tempfile::tempdir().expect("config root");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = config_root.path().join("configs").join("config.toml");
    let mut config = AppConfig::default();
    config.modules.patch = "null".to_owned();
    let runtime = AgentRuntime::builder(config, workspace.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .expect("runtime");
    replace_workflow_for_test(&runtime, Arc::new(ShortHistoryWorkflow)).await;

    let error = runtime
        .run("current request".to_owned())
        .await
        .expect_err("bad workflow must fail");

    assert!(
        error
            .to_string()
            .contains("workflow returned no new persistent turn messages")
    );
    let history = runtime.history().await;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].role, MessageRole::User);
    assert_eq!(message_text_for_test(&history[0]), "current request");
    let stored = runtime
        .session
        .session_store
        .as_ref()
        .expect("session store")
        .load_messages()
        .expect("load messages");
    assert_eq!(stored, history);
}

#[tokio::test]
async fn compaction_replaces_runtime_and_session_history() {
    let config_root = tempfile::tempdir().expect("config root");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = config_root.path().join("configs").join("config.toml");
    let mut config = AppConfig::default();
    config.modules.patch = "null".to_owned();
    let runtime = AgentRuntime::builder(config, workspace.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .expect("runtime");

    replace_workflow_for_test(&runtime, Arc::new(CompactingWorkflow)).await;
    {
        let mut history = runtime.session.history.lock().await;
        history.push(CanonicalMessage::text(MessageRole::User, "old request"));
        history.push(CanonicalMessage::text(MessageRole::Assistant, "old answer"));
    }
    let seed_history = runtime.session.history.lock().await.clone();
    runtime
        .session
        .session_store
        .as_ref()
        .expect("session store")
        .append_history(runtime.session.thread_id, None, &seed_history)
        .await
        .expect("seed session store");

    runtime.run("current request".to_owned()).await.unwrap();

    let history = runtime.history().await;
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].role, MessageRole::User);
    assert!(message_text_for_test(&history[0]).contains("compacted summary"));
    assert!(message_text_for_test(&history[1]).contains("current request"));
    assert!(message_text_for_test(&history[2]).contains("done after compact"));

    let stored = runtime
        .session
        .session_store
        .as_ref()
        .expect("session store")
        .load_messages()
        .expect("load replaced messages");
    assert_eq!(stored, history);
}

#[tokio::test]
async fn model_exchange_is_recorded_in_session_journal() {
    let config_root = tempfile::tempdir().expect("config root");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = config_root.path().join("configs").join("config.toml");
    let mut config = AppConfig::default();
    config.modules.patch = "null".to_owned();
    let runtime = AgentRuntime::builder(config, workspace.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .expect("runtime");
    replace_workflow_for_test(&runtime, Arc::new(ModelCallingWorkflow)).await;

    runtime.run("record exchange".to_owned()).await.unwrap();

    let records = runtime
        .session
        .session_store
        .as_ref()
        .expect("session store")
        .load_records()
        .expect("journal records");
    assert!(records.iter().any(|record| matches!(
        record.entry,
        crate::core::JournalEntry::ModelRequestRecorded(_)
    )));
    assert!(records.iter().any(|record| matches!(
        record.entry,
        crate::core::JournalEntry::ModelResponseRecorded(_)
    )));
}

#[tokio::test]
async fn runtime_writes_config_snapshot_when_session_is_persisted() {
    let config_root = tempfile::tempdir().expect("config root");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = config_root.path().join("configs").join("config.toml");
    let mut config = AppConfig::default();
    config.profile.name = "snapshot-profile".to_owned();
    config.modules.workflow = "coding.plan_execute_review".to_owned();
    config.modules.context = "simple".to_owned();
    config.modules.compactor = "none".to_owned();
    config.modules.tool_exposure = "all_visible".to_owned();
    let runtime = AgentRuntime::builder(config, workspace.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .expect("runtime");
    runtime.start_session().await.expect("start session");
    assert!(!runtime.session_dir().expect("session dir").exists());
    replace_workflow_for_test(&runtime, Arc::new(DelayedWorkflow)).await;
    runtime.run("persist session".to_owned()).await.unwrap();

    let snapshot_path = runtime
        .session_dir()
        .expect("session dir")
        .join(crate::core::CONFIG_SNAPSHOT_FILE);
    let value: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(snapshot_path).expect("read config snapshot"),
    )
    .expect("config snapshot json");

    assert_eq!(value["schema_version"], 3);
    assert_eq!(value["active_provider"], "fake");
    assert_eq!(value["profile_name"], "snapshot-profile");
    assert_eq!(value["modules"]["workflow"], "coding.plan_execute_review");
    assert_eq!(value["modules"]["context"], "simple");
    assert_eq!(value["modules"]["compactor"], "none");
    assert_eq!(value["modules"]["tool_exposure"], "all_visible");
    assert!(value["tools"].as_array().is_some());

    let mut reloaded_config = AppConfig::default();
    reloaded_config.profile.name = "reloaded-profile".to_owned();
    reloaded_config.modules.patch = "null".to_owned();
    let mut reloaded_registry = BuiltinRegistry::from_catalog(
        &reloaded_config,
        workspace.path().to_path_buf(),
        test_catalog(),
    )
    .expect("reloaded registry");
    reloaded_registry.workflow = Arc::new(DelayedWorkflow);
    let reloaded_snapshot =
        SessionConfigSnapshot::from_runtime_config(&reloaded_config, &reloaded_registry);
    runtime
        .reload_registry(reloaded_registry, Some(reloaded_snapshot))
        .await
        .expect("reload registry");
    runtime
        .set_model_name("runtime-model-override".to_owned())
        .await;
    runtime.set_reasoning_effort(Some("high".to_owned())).await;
    runtime.run("after reload".to_owned()).await.unwrap();

    let records = runtime
        .session
        .session_store
        .as_ref()
        .expect("session store")
        .load_records()
        .expect("journal records");
    let (opened, module_epoch) = records
        .iter()
        .rev()
        .find_map(|record| match &record.entry {
            crate::core::JournalEntry::TurnOpened(opened) => Some((opened, opened.module_epoch)),
            _ => None,
        })
        .expect("latest turn_opened");
    let turn_snapshot: SessionConfigSnapshot =
        serde_json::from_value(opened.config_snapshot.clone()).expect("turn config snapshot");
    assert_eq!(module_epoch, 1);
    assert_eq!(turn_snapshot.profile_name, "reloaded-profile");
    assert_eq!(turn_snapshot.model.model, "runtime-model-override");
    assert_eq!(turn_snapshot.reasoning.effort.as_deref(), Some("high"));
}

#[tokio::test]
async fn run_errors_when_workflow_timeout_is_reached() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let mut config = AppConfig::default();
    config.runtime.workflow_timeout_ms = 50;
    let runtime = AgentRuntime::builder(config, cwd.path().to_path_buf())
        .with_module_catalog(test_catalog())
        .build()
        .expect("runtime");
    replace_workflow_for_test(&runtime, Arc::new(HangingWorkflow)).await;

    let error = runtime
        .run("current".to_owned())
        .await
        .expect_err("hung workflow must time out");

    assert!(error.to_string().contains("workflow timed out after 50ms"));
}

#[tokio::test]
async fn workflow_timeout_zero_disables_runtime_timeout() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let mut config = AppConfig::default();
    config.runtime.workflow_timeout_ms = 0;
    let runtime = AgentRuntime::builder(config, cwd.path().to_path_buf())
        .with_module_catalog(test_catalog())
        .build()
        .expect("runtime");
    replace_workflow_for_test(&runtime, Arc::new(DelayedWorkflow)).await;

    let output = runtime.run("current".to_owned()).await.unwrap();

    assert_eq!(output.text, "done");
}

#[tokio::test]
async fn reload_registry_publishes_new_snapshot_without_mutating_running_turn() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let config = AppConfig::default();
    let runtime = Arc::new(
        AgentRuntime::builder(config, cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime"),
    );
    let workflow = Arc::new(SnapshotProbeWorkflow {
        wait_once: Arc::new(AtomicBool::new(true)),
        started: Arc::new(tokio::sync::Notify::new()),
        proceed: Arc::new(tokio::sync::Notify::new()),
    });
    replace_workflow_for_test(&runtime, workflow.clone()).await;

    let running_runtime = runtime.clone();
    let running = tokio::spawn(async move { running_runtime.run("probe".to_owned()).await });
    workflow.started.notified().await;

    let mut next_config = AppConfig::default();
    next_config.tools.configured.push(ConfiguredToolConfig {
        name: "late_tool".to_owned(),
        description: "Appears after reload".to_owned(),
        input_schema: serde_json::json!({"type": "object"}),
        surface: crate::domain::ToolSurface::default(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: None,
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Process {
            command: "printf".to_owned(),
            args: vec!["ok".to_owned()],
            environment: Default::default(),
        },
    });
    let next_registry =
        BuiltinRegistry::from_catalog(&next_config, cwd.path().to_path_buf(), test_catalog())
            .expect("next registry");
    let report = runtime
        .reload_registry(next_registry, None)
        .await
        .expect("reload registry");
    assert_eq!(report.old_epoch, 0);
    assert_eq!(report.new_epoch, 1);
    assert!(report.tool_names.iter().any(|name| name == "late_tool"));

    workflow.proceed.notify_one();
    let running_output = running
        .await
        .expect("running task")
        .expect("running output");
    assert_eq!(running_output.text, "has_late_tool=false");

    replace_workflow_for_test(&runtime, workflow).await;
    let next_output = runtime.run("probe after reload".to_owned()).await.unwrap();
    assert_eq!(next_output.text, "has_late_tool=true");
}
