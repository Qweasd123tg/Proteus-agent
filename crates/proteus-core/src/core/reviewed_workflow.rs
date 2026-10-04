//! Root-owned completion review, shared by live execution and journal replay.
use crate::{
    contracts::{
        AgentWorkflowContext, EventSink, ExecutionAttribution, HookEvent, HookInput, Workflow,
        WorkflowContinuation, WorkflowHistoryCheckpoint, WorkflowHistoryUpdate, WorkflowOutput,
    },
    domain::{AgentTask, Event, EventEnvelope},
    model_standard::{CanonicalMessage, InstructionBlock, InstructionKind},
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::sync::Arc;

const MAX_CONTINUATIONS: u32 = 8;

/// Host-owned delivered-input normalization. It observes accepted deliveries;
/// it never admits queued messages at a completion boundary.
#[async_trait]
pub(crate) trait CandidateHistoryNormalizer: Send + Sync {
    async fn normalize(&self, output: &mut WorkflowOutput) -> Result<()>;
}

// Workflows can emit candidate finals. Only an accepted final reaches the UI.
struct ReviewedEvents(Arc<crate::contracts::EventEmitter>);
#[async_trait]
impl EventSink for ReviewedEvents {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        if !matches!(envelope.event, Event::TurnFinished { .. }) {
            self.0
                .emit(
                    crate::domain::EventContext::new(
                        envelope.session_id,
                        envelope.thread_id,
                        envelope.turn_id,
                    ),
                    envelope.event,
                )
                .await?;
        }
        Ok(())
    }
}

pub(super) async fn run(
    workflow: &dyn Workflow,
    task: AgentTask,
    initial_history: Vec<CanonicalMessage>,
    context: AgentWorkflowContext,
    normalizer: Option<&dyn CandidateHistoryNormalizer>,
) -> Result<WorkflowOutput> {
    if !context.execution.hooks.is_active() {
        return workflow.run(task, initial_history, context.into()).await;
    }
    let mut history = initial_history.clone();
    let original_user_message_id = history
        .last()
        .context("completion review has no persisted user anchor")?
        .id;
    let mut continuation = None;
    let events = Arc::new(crate::contracts::EventEmitter::new(Arc::new(
        ReviewedEvents(context.events.clone()),
    )));
    for attempt in 0..=MAX_CONTINUATIONS {
        if context.execution.is_cancelled() {
            bail!("workflow execution canceled");
        }
        let mut invocation = context.clone();
        invocation.events = events.clone();
        invocation.continuation = continuation;
        if let Some(review) = &invocation.continuation {
            invocation.instructions.push(InstructionBlock::new(
                InstructionKind::Developer,
                review.reason.clone(),
                128,
            ));
        }
        let mut output = workflow
            .run(task.clone(), history, invocation.into())
            .await?;
        if let Some(normalizer) = normalizer {
            normalizer.normalize(&mut output).await?;
        }
        let progress = WorkflowHistoryUpdate {
            new_messages: output.new_messages.clone(),
            history_replacement: output.history_replacement.clone(),
            compactions: output.compactions.clone(),
        };
        // Validate and commit the candidate before waiting on its review. A
        // timeout, failed reviewer or cancellation retains this known progress.
        context
            .history_recorder
            .checkpoint(WorkflowHistoryCheckpoint {
                history: progress.clone(),
                tool_results: vec![],
            })
            .await?;
        history = output
            .history_replacement
            .clone()
            .unwrap_or_else(|| initial_history.clone());
        history.extend(output.new_messages.clone());
        let mut current_user_message_id = original_user_message_id;
        for report in &output.compactions {
            for replacement in &report.user_message_replacements {
                if replacement.source_message_id == current_user_message_id {
                    current_user_message_id = replacement.replacement_message_id;
                }
            }
        }
        let reviewed = context
            .execution
            .hooks
            .apply(HookInput {
                event: HookEvent::BeforeStop {
                    task: task.clone(),
                    history: history.clone(),
                    output: output.output.clone(),
                    attempt,
                    continuation: None,
                },
                attribution: ExecutionAttribution::for_turn(
                    context.execution.scope.execution_id,
                    context.session_id,
                    context.thread_id,
                    context.turn_id,
                ),
                cwd: task.cwd.clone(),
            })
            .await?;
        let HookEvent::BeforeStop {
            continuation: reason,
            ..
        } = reviewed
        else {
            bail!("hook changed completion review event kind");
        };
        if context.execution.is_cancelled() {
            bail!("workflow execution canceled");
        }
        let Some(reason) = reason else {
            context
                .events
                .emit(
                    crate::domain::EventContext::new(
                        context.session_id,
                        context.thread_id,
                        Some(context.turn_id),
                    ),
                    Event::TurnFinished {
                        output: output.output.clone(),
                    },
                )
                .await?;
            return Ok(output);
        };
        if attempt == MAX_CONTINUATIONS {
            bail!("before_stop continuation limit exceeded ({MAX_CONTINUATIONS})");
        }
        continuation = Some(WorkflowContinuation {
            attempt: attempt + 1,
            reason,
            current_user_message_id,
            history: progress,
        });
    }
    unreachable!("bounded completion review loop always settles")
}
