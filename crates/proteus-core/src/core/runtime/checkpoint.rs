use std::sync::Arc;

use anyhow::{Result, ensure};
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{
    contracts::{
        ExecutionAttribution, ToolExecutionRecorder, WorkflowHistoryCheckpoint,
        WorkflowHistoryRecorder,
    },
    core::{SessionStore, session_journal::history_capture::HistoryCapture},
    domain::{ToolCall, ToolCallResolution, ToolResult},
    model_standard::CanonicalMessage,
};

use super::{
    prepare_failed_history_update,
    steering::{SteeringModel, weave_deliveries_into_failed_history},
};

/// Per-invocation binding. The same object records tool facts and explicit
/// workflow checkpoints; neither module identity nor component location matters.
pub(super) struct TurnHistoryRecorder {
    pub(super) attribution: ExecutionAttribution,
    pub(super) store: Option<SessionStore>,
    pub(super) history: Arc<Mutex<Vec<CanonicalMessage>>>,
    pub(super) model_context: Arc<Mutex<crate::core::model_context::ModelContextState>>,
    pub(super) interrupted_turns: Arc<Mutex<Vec<crate::contracts::WorkflowHistoryInterruption>>>,
    pub(super) initial_history: Vec<CanonicalMessage>,
    pub(super) current_user: CanonicalMessage,
    pub(super) steering: SteeringModel,
    pub(super) tools: Arc<dyn ToolExecutionRecorder>,
    pub(super) capture: Mutex<HistoryCapture>,
    pub(super) recorded_compactions: Mutex<usize>,
    pub(super) pending_effects: Mutex<Vec<ToolResult>>,
}

impl TurnHistoryRecorder {
    pub(super) async fn settle_in_memory_effects(&self) -> Result<()> {
        if self.store.is_some() {
            return Ok(());
        }
        let mut capture = self.capture.lock().await;
        let mut effects = self.pending_effects.lock().await;
        if effects.is_empty() {
            return Ok(());
        }
        let mut history = self.history.lock().await;
        for result in effects.drain(..) {
            capture.record(&mut history, &result)?;
        }
        Ok(())
    }
}

#[async_trait]
impl WorkflowHistoryRecorder for TurnHistoryRecorder {
    async fn checkpoint(&self, mut checkpoint: WorkflowHistoryCheckpoint) -> Result<()> {
        let deliveries = self.steering.delivery_records().await;
        let allowed = weave_deliveries_into_failed_history(&mut checkpoint.history, &deliveries)?;
        let progress = checkpoint.history;
        let prepared = prepare_failed_history_update(
            &self.initial_history,
            &self.current_user,
            &progress.new_messages,
            progress.history_replacement.as_deref(),
            &progress.compactions,
            &allowed,
        )?;
        let mut capture = self.capture.lock().await;
        let mut recorded_compactions = self.recorded_compactions.lock().await;
        let mut history = self.history.lock().await;
        ensure!(
            progress.compactions.len() >= *recorded_compactions,
            "checkpoint discarded previously reported compactions"
        );
        let compaction = progress
            .compactions
            .iter()
            .skip(*recorded_compactions)
            .rev()
            .find(|report| report.changed)
            .cloned();
        let (next_capture, next_history) = capture.rebase(
            &history,
            &prepared.final_messages,
            &checkpoint.tool_results,
            compaction.is_some(),
        )?;
        if let Some(store) = &self.store {
            let agent = self.attribution.agent.expect("root turn attribution");
            store
                .checkpoint_history(
                    agent.thread_id,
                    agent.turn_id,
                    &prepared.final_messages,
                    compaction.clone(),
                    checkpoint.tool_results,
                )
                .await?;
        }
        if compaction.is_some() {
            self.model_context.lock().await.compacted();
            self.interrupted_turns.lock().await.clear();
        }
        *history = next_history;
        *capture = next_capture;
        *recorded_compactions = progress.compactions.len();
        Ok(())
    }
}

#[async_trait]
impl ToolExecutionRecorder for TurnHistoryRecorder {
    async fn tool_effect_recorded(
        &self,
        attribution: ExecutionAttribution,
        result: &ToolResult,
    ) -> Result<()> {
        self.tools.tool_effect_recorded(attribution, result).await?;
        if self.store.is_none() && attribution == self.attribution {
            self.pending_effects.lock().await.push(result.clone());
        }
        Ok(())
    }

    async fn tool_call_requested(
        &self,
        attribution: ExecutionAttribution,
        call: &ToolCall,
    ) -> Result<()> {
        if attribution == self.attribution {
            self.capture.lock().await.validate_call(call)?;
        }
        self.tools.tool_call_requested(attribution, call).await
    }

    async fn tool_call_resolved(
        &self,
        attribution: ExecutionAttribution,
        call: &ToolCall,
        resolution: &ToolCallResolution,
    ) -> Result<()> {
        self.tools
            .tool_call_resolved(attribution, call, resolution)
            .await
    }

    async fn tool_approval_requested(
        &self,
        attribution: ExecutionAttribution,
        call: &ToolCall,
        reason: &str,
    ) -> Result<()> {
        self.tools
            .tool_approval_requested(attribution, call, reason)
            .await
    }

    async fn tool_result_recorded(
        &self,
        attribution: ExecutionAttribution,
        result: &ToolResult,
    ) -> Result<()> {
        let mut capture = self.capture.lock().await;
        self.tools.tool_result_recorded(attribution, result).await?;
        if attribution == self.attribution {
            let mut effects = self.pending_effects.lock().await;
            capture.record(&mut *self.history.lock().await, result)?;
            // No cancellation point between accepting final and retiring raw.
            effects.retain(|effect| effect.call_id != result.call_id);
        }
        Ok(())
    }
}
