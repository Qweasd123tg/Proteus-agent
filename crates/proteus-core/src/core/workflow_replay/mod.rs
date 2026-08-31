use std::{collections::HashSet, path::Path, sync::Arc, time::Instant};

use anyhow::{Context, Result};

use crate::{
    contracts::{
        AgentWorkflowContext, CancellationToken, EventEmitter, ExecutionContext, ExecutionScope,
        NoopExecutionRecorder,
    },
    core::{
        AppConfig, BoundModel, HeadlessUserInputTransport, InMemoryEventStore, ModeAwarePolicy,
        ModelExecutionBinding, ModelService, ModuleBuildContext, ModuleCatalog, PolicyBuildContext,
        TurnSettlementStatus, prepare_history_update,
    },
    stubs::{NoMemory, NullSearch},
};

mod fixture;
mod normalize;
mod replay_runtime;
mod report;

pub use report::*;

use fixture::load_fixture;
use normalize::changed_compactions_equal;
use replay_runtime::{
    ReplayApprovalTransport, ReplayCompactor, ReplayContextBuilder, ReplayModel, ReplayState,
    ReplayToolExposure, register_replay_tools,
};

/// Replays one recorded root turn through its selected Workflow module.
///
/// Model responses, context, compaction, tool exposure and tool results come
/// exclusively from the canonical journal. The source journal is opened
/// read-only for the run and no provider adapter or real tool implementation
/// is constructed.
pub async fn replay_workflow(
    path: impl AsRef<Path>,
    config: &AppConfig,
    catalog: &ModuleCatalog,
    options: WorkflowReplayOptions,
) -> Result<WorkflowReplayReport> {
    let fixture = load_fixture(path.as_ref(), options)?;
    let workflow_id = fixture
        .snapshot
        .modules
        .workflow
        .clone()
        .context("recorded turn has no workflow module")?;
    let policy_id = fixture
        .snapshot
        .modules
        .policy
        .clone()
        .context("recorded turn has no policy module")?;
    let journal_before = std::fs::read(&fixture.journal_path).with_context(|| {
        format!(
            "failed to read source journal {} before workflow replay",
            fixture.journal_path.display()
        )
    })?;
    let started = Instant::now();

    let specs = fixture
        .snapshot
        .tools
        .iter()
        .map(|tool| tool.spec.clone())
        .collect::<Vec<_>>();
    let registered_tool_names = specs
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<HashSet<_>>();
    let state = Arc::new(ReplayState::new(
        fixture.exchanges.clone(),
        fixture.tools.clone(),
        fixture.compactions.clone(),
        fixture.context.clone(),
        registered_tool_names,
        &fixture.snapshot.reasoning,
    ));
    let tools = register_replay_tools(state.clone(), specs)?;

    let mut replay_config = config.clone();
    replay_config.profile.name = fixture.snapshot.profile_name.clone();
    replay_config.modules = fixture.snapshot.modules.clone();
    replay_config.permissions.mode = fixture.snapshot.permission_mode_default;
    let context_providers = catalog.build_context_providers(&fixture.opened.task.cwd)?;
    let build_ctx = ModuleBuildContext {
        config: &replay_config,
        cwd: &fixture.opened.task.cwd,
        context_providers: &context_providers,
    };
    let workflow = catalog
        .build_workflow(&workflow_id, &build_ctx)
        .with_context(|| format!("failed to build recorded workflow module '{}'", workflow_id))?;
    let policy_ctx = PolicyBuildContext {
        cwd: &fixture.opened.task.cwd,
    };
    let policy = catalog
        .build_policy(&policy_id, &policy_ctx)
        .with_context(|| format!("failed to build recorded policy module '{}'", policy_id))?;
    let policy = Arc::new(ModeAwarePolicy::new(
        fixture.snapshot.permission_mode_default,
        policy,
    ));

    let event_store = Arc::new(InMemoryEventStore::new());
    let events = Arc::new(EventEmitter::new(event_store));
    let model_service = Arc::new(ModelService::new(Arc::new(ReplayModel::new(state.clone()))));
    let scope = ExecutionScope::fresh(CancellationToken::new());
    let model_binding = ModelExecutionBinding::for_turn(
        scope.clone(),
        events.clone(),
        fixture.session_id,
        fixture.thread_id,
        fixture.turn_id,
        Arc::new(NoopExecutionRecorder),
    );
    let model: Arc<dyn crate::contracts::Model> =
        Arc::new(BoundModel::new(model_service, model_binding));
    let approval: Arc<dyn crate::contracts::ApprovalTransport> =
        Arc::new(ReplayApprovalTransport::new(state.clone()));
    let execution_context = ExecutionContext::new(
        scope,
        0,
        model,
        Arc::new(NullSearch),
        Arc::new(NoMemory),
        tools,
        policy,
        approval,
    );
    let workflow_context = AgentWorkflowContext::new(
        execution_context,
        fixture.session_id,
        fixture.thread_id,
        fixture.turn_id,
        fixture.snapshot.model.clone(),
        fixture.snapshot.reasoning.clone(),
        replay_config.runtime.context_timeout_ms.max(1),
        events,
        Arc::new(ReplayContextBuilder::new(state.clone())),
        Arc::new(HeadlessUserInputTransport),
        Arc::new(ReplayCompactor::new(state.clone())),
        Arc::new(ReplayToolExposure::new(state.clone())),
        None,
    )
    .with_tool_recorder(state.clone())
    .with_instructions(replay_config.instruction_blocks());

    let replay_result = workflow
        .run(
            fixture.opened.task.clone(),
            fixture.initial_history.clone(),
            workflow_context,
        )
        .await
        .and_then(|output| {
            let persisted_user = fixture
                .initial_history
                .last()
                .context("workflow replay fixture has no persisted current user message")?;
            let history_compacted = output.compactions.iter().any(|report| report.changed);
            let history_update = prepare_history_update(
                &fixture.initial_history,
                persisted_user,
                &output.new_messages,
                output.history_replacement.as_deref(),
                history_compacted,
                &HashSet::new(),
            )?;
            Ok((
                output.output,
                history_update.final_messages,
                output.compactions,
            ))
        });
    let (replay_outcome, replay_history, replay_compactions) = match replay_result {
        Ok((output, history, compactions)) => (
            WorkflowReplayOutcome {
                status: TurnSettlementStatus::Success,
                output: Some(output),
                error: None,
            },
            Some(history),
            Some(compactions),
        ),
        Err(error) => (
            WorkflowReplayOutcome {
                status: TurnSettlementStatus::Error,
                output: None,
                error: Some(format!("{error:#}")),
            },
            Some(fixture.initial_history.clone()),
            None,
        ),
    };
    let summary = state.summary();
    let journal_after = std::fs::read(&fixture.journal_path).with_context(|| {
        format!(
            "failed to read source journal {} after workflow replay",
            fixture.journal_path.display()
        )
    })?;
    let source_journal_unchanged = journal_before == journal_after;

    let recorded_outcome = WorkflowReplayOutcome {
        status: fixture.settlement.status,
        output: fixture.settlement.output.clone(),
        error: fixture.settlement.error.clone(),
    };
    let settlement_equal = recorded_outcome.status == replay_outcome.status;
    let output_equal = match (&recorded_outcome.output, &replay_outcome.output) {
        (None, None) => None,
        (Some(recorded), Some(replay)) => Some(state.outputs_equal(replay, recorded)),
        _ => Some(false),
    };
    let error_equal = optional_equality(&recorded_outcome.error, &replay_outcome.error);
    let history_equal = replay_history
        .as_deref()
        .map(|history| state.histories_equal(history, &fixture.final_history));
    let compactions_equal = replay_compactions
        .as_deref()
        .map(|reports| changed_compactions_equal(reports, &fixture.compactions));
    let mut issues = summary.issues;
    if !settlement_equal {
        issues.push(format!(
            "turn settlement differs: recorded={:?}, replay={:?}",
            recorded_outcome.status, replay_outcome.status
        ));
    }
    if output_equal == Some(false) {
        issues.push("workflow output differs from recorded turn output".to_owned());
    }
    if error_equal == Some(false) {
        issues.push("workflow error differs from recorded turn error".to_owned());
    }
    if history_equal == Some(false) {
        issues.push(
            "workflow persistent history differs from the recorded journal projection".to_owned(),
        );
    }
    if compactions_equal == Some(false) {
        issues.push(
            "workflow changed compaction reports differ from the canonical journal".to_owned(),
        );
    }
    if !source_journal_unchanged {
        issues.push("source journal changed during workflow replay".to_owned());
    }
    let matched = issues.is_empty()
        && settlement_equal
        && output_equal != Some(false)
        && error_equal != Some(false)
        && history_equal != Some(false)
        && compactions_equal != Some(false)
        && source_journal_unchanged;

    Ok(WorkflowReplayReport {
        schema_version: WORKFLOW_REPLAY_REPORT_SCHEMA_VERSION,
        source: WorkflowReplaySource {
            journal_path: fixture.journal_path,
            session_id: fixture.session_id,
            thread_id: fixture.thread_id,
            turn_id: fixture.turn_id,
            module_epoch: fixture.opened.module_epoch,
            profile_name: fixture.snapshot.profile_name,
            workflow_id,
            policy_id,
        },
        recorded: recorded_outcome,
        replay: replay_outcome,
        model_exchanges: WorkflowReplayCounts {
            recorded: fixture.exchanges.len(),
            replayed: summary.model_exchanges,
        },
        tool_calls: WorkflowReplayCounts {
            recorded: fixture.tools.len(),
            replayed: summary.tool_calls,
        },
        comparison: WorkflowReplayComparison {
            matched,
            settlement_equal,
            output_equal,
            error_equal,
            history_equal,
            issues,
        },
        source_journal_unchanged,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

fn optional_equality<T: PartialEq>(left: &Option<T>, right: &Option<T>) -> Option<bool> {
    match (left, right) {
        (None, None) => None,
        _ => Some(left == right),
    }
}

#[cfg(test)]
mod tests;
