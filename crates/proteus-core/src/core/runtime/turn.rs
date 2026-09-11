use std::{error::Error as StdError, fmt, sync::Arc};

use anyhow::Result;
use tokio::time::{Duration, timeout};

use crate::{
    contracts::{CancellationToken, ExecutionAttribution, ExecutionScope},
    core::SessionConfigSnapshot,
    domain::{AgentOutput, AgentTask, Event, EventContext},
    model_standard::CanonicalMessage,
};

use super::{
    AgentRuntime, ExecutionAdmissionSnapshot, ReservedRunCompletion, prepare_history_update,
    steering::{
        ReservedUserMessage, RootTurnSettlement, SteeringModel, UserMessageReservation,
        weave_deliveries_into_output,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnAbort {
    Canceled,
    WorkflowTimeout { timeout_ms: u64 },
}

impl fmt::Display for TurnAbort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Canceled => formatter.write_str("turn canceled by client"),
            Self::WorkflowTimeout { timeout_ms } => {
                write!(formatter, "workflow timed out after {timeout_ms}ms")
            }
        }
    }
}

impl StdError for TurnAbort {}

impl AgentRuntime {
    pub async fn run(&self, text: String) -> Result<AgentOutput> {
        self.run_with_cancellation(text, CancellationToken::new())
            .await
    }

    pub async fn run_with_cancellation(
        &self,
        text: String,
        cancellation: CancellationToken,
    ) -> Result<AgentOutput> {
        self.run_completion(text, cancellation).await?.into_result()
    }

    pub(crate) async fn run_completion(
        &self,
        text: String,
        cancellation: CancellationToken,
    ) -> Result<ReservedRunCompletion> {
        let _run_guard = self.session.run_lock.lock().await;
        let reserved = match self.reserve_user_message(text).await? {
            UserMessageReservation::Start(reserved) => reserved,
            UserMessageReservation::Queued(_) => {
                anyhow::bail!("session acquired the run lock with an active root reservation")
            }
        };
        Ok(self.run_reserved_chain(reserved, cancellation).await)
    }

    /// Atomically reserves an idle root session or appends to its bounded
    /// steering queue. App-server transports call this before spawning a turn,
    /// eliminating the race between the first and second `Send` commands.
    pub(crate) async fn reserve_user_message(
        &self,
        text: String,
    ) -> Result<UserMessageReservation> {
        self.reserve_user_message_with_options(text, crate::domain::RunOptions::default())
            .await
    }

    pub(crate) async fn reserve_user_message_with_options(
        &self,
        text: String,
        options: crate::domain::RunOptions,
    ) -> Result<UserMessageReservation> {
        if let Some(intent) = &options.intent {
            anyhow::ensure!(
                !intent.is_empty()
                    && intent.len() <= 128
                    && intent
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._/-".contains(&c)),
                "invalid workflow intent name"
            );
        }
        let snapshot = self.capture_run_snapshot(options.permission_mode).await;
        let mut reservation = self
            .session
            .steering
            .reserve_with_options(text, options)
            .await?;
        if let UserMessageReservation::Start(reserved) = &mut reservation {
            reserved.snapshot = Some(snapshot);
        }
        if let UserMessageReservation::Queued(receipt) = &reservation {
            self.services
                .events
                .emit(
                    EventContext::new(
                        self.session.session_id,
                        self.session.thread_id,
                        Some(receipt.active_turn_id),
                    ),
                    Event::SteeringQueued {
                        message_id: receipt.message_id,
                        text: receipt.text.clone(),
                        queued_count: receipt.queued_count,
                    },
                )
                .await?;
        }
        Ok(reservation)
    }

    #[cfg(test)]
    pub(crate) async fn run_reserved_with_cancellation(
        &self,
        reserved: ReservedUserMessage,
        cancellation: CancellationToken,
    ) -> Result<AgentOutput> {
        self.run_reserved_completion(reserved, cancellation)
            .await?
            .into_result()
    }

    pub(crate) async fn run_reserved_completion(
        &self,
        reserved: ReservedUserMessage,
        cancellation: CancellationToken,
    ) -> Result<ReservedRunCompletion> {
        let _run_guard = self.session.run_lock.lock().await;
        self.session
            .steering
            .validate_reservation(reserved.turn_id)
            .await?;
        Ok(self.run_reserved_chain(reserved, cancellation).await)
    }

    async fn run_reserved_chain(
        &self,
        mut reserved: ReservedUserMessage,
        cancellation: CancellationToken,
    ) -> ReservedRunCompletion {
        let mut reservation_guard = self.session.steering.run_guard();
        let settled = async {
            loop {
                let turn_id = reserved.turn_id;
                let snapshot = reserved.snapshot.clone();
                let intent = reserved.intent.clone();
                let output = self.run_one_turn(reserved, cancellation.clone()).await?;
                if cancellation.is_cancelled() {
                    anyhow::bail!("turn canceled by client");
                }
                match self
                    .session
                    .steering
                    .settle_and_take_followup(turn_id)
                    .await?
                {
                    RootTurnSettlement::FollowUp(mut followup) => {
                        followup.snapshot = snapshot;
                        followup.intent = intent;
                        reserved = followup;
                    }
                    RootTurnSettlement::Complete(finalization) => {
                        return Ok((output, finalization));
                    }
                }
            }
        }
        .await;

        let (result, finalization) = match settled {
            Ok((output, finalization)) => (Ok(output), finalization),
            Err(error) => {
                let finalization = self.session.steering.finalization_guard().await;
                (Err(error), finalization)
            }
        };
        reservation_guard.disarm();
        ReservedRunCompletion {
            result,
            _finalization: finalization,
        }
    }

    async fn run_one_turn(
        &self,
        mut reserved: ReservedUserMessage,
        cancellation: CancellationToken,
    ) -> Result<AgentOutput> {
        let snapshot = match reserved.snapshot.take() {
            Some(snapshot) => snapshot,
            None => self.capture_execution_snapshot().await,
        };
        let execution_scope = ExecutionScope::fresh(cancellation.clone());
        self.ensure_session_started_with_snapshot(&snapshot).await?;
        let turn_id = reserved.turn_id;
        let execution_attribution = ExecutionAttribution::for_turn(
            execution_scope.execution_id,
            self.session.session_id,
            self.session.thread_id,
            turn_id,
        );
        let task = AgentTask::new(reserved.text.clone(), self.services.cwd.clone());
        if cancellation.is_cancelled() {
            return Err(TurnAbort::Canceled.into());
        }
        let config_snapshot = snapshot.config_snapshot.clone();
        if let Some(session_store) = &self.session.session_store {
            let base_history_revision = session_store.load_projection()?.history_revision;
            session_store
                .append_execution_journal_entry(
                    execution_attribution,
                    crate::core::JournalEntry::TurnOpened(crate::core::TurnOpened {
                        intent: reserved.intent.clone(),
                        task: task.clone(),
                        base_history_revision,
                        module_epoch: snapshot.runtime.epoch.as_u64(),
                        config_snapshot: serde_json::to_value(config_snapshot.as_ref())?,
                    }),
                )
                .await?;
        }
        let result = self
            .run_opened_turn(reserved, execution_scope, snapshot, config_snapshot, task)
            .await;
        let settlement = match &result {
            Ok(output) => crate::core::TurnSettled {
                status: crate::core::TurnSettlementStatus::Success,
                output: Some(output.clone()),
                error: None,
            },
            Err(error) => {
                let message = format!("{error:#}");
                let status = turn_settlement_status(error, cancellation.is_cancelled());
                crate::core::TurnSettled {
                    status,
                    output: None,
                    error: Some(message),
                }
            }
        };
        if let Some(session_store) = &self.session.session_store
            && let Err(settlement_error) = session_store
                .append_journal_entry(
                    self.session.thread_id,
                    Some(turn_id),
                    crate::core::JournalEntry::TurnSettled(settlement),
                )
                .await
        {
            return match result {
                Ok(_) => Err(settlement_error
                    .context("turn completed but its canonical settlement could not be persisted")),
                Err(turn_error) => Err(anyhow::anyhow!(
                    "{turn_error:#}; additionally failed to persist turn settlement: {settlement_error:#}"
                )),
            };
        }
        result
    }

    async fn run_opened_turn(
        &self,
        reserved: ReservedUserMessage,
        execution_scope: ExecutionScope,
        snapshot: ExecutionAdmissionSnapshot,
        config_snapshot: Option<SessionConfigSnapshot>,
        task: AgentTask,
    ) -> Result<AgentOutput> {
        let turn_id = reserved.turn_id;
        let cancellation = execution_scope.cancellation.clone();
        let user_message = reserved.message;
        let event_context = EventContext::new(
            self.session.session_id,
            self.session.thread_id,
            Some(turn_id),
        );
        self.services
            .events
            .emit(
                event_context,
                Event::TurnStarted {
                    session_id: self.session.session_id,
                    thread_id: self.session.thread_id,
                    turn_id,
                },
            )
            .await?;
        let history = self
            .persist_current_user_message(turn_id, &user_message, config_snapshot.as_ref())
            .await?;
        if let Some(kind) = reserved.delivery {
            self.services
                .events
                .emit(
                    EventContext::new(
                        self.session.session_id,
                        self.session.thread_id,
                        Some(turn_id),
                    ),
                    Event::SteeringDelivered {
                        message_id: user_message.id,
                        text: task.text.clone(),
                        kind,
                        queued_count: self
                            .session
                            .steering
                            .queued_count_handle()
                            .load(std::sync::atomic::Ordering::Acquire),
                    },
                )
                .await?;
        }
        let mut workflow_context =
            self.bind_agent_workflow_context(execution_scope, &snapshot, turn_id);
        workflow_context.queued_user_messages = self.session.steering.queued_count_handle();
        workflow_context.intent = reserved.intent;
        workflow_context.permission_mode = snapshot.permission_mode;
        let steering_model = SteeringModel::new(
            workflow_context.execution.model.clone(),
            self.session.steering.clone(),
            self.services.events.clone(),
            self.session.session_id,
            self.session.thread_id,
            turn_id,
        );
        workflow_context.execution.model = Arc::new(steering_model.clone());
        let checkpoint_recorder = Arc::new(super::checkpoint::TurnHistoryRecorder {
            attribution: ExecutionAttribution::for_turn(
                workflow_context.execution.scope.execution_id,
                self.session.session_id,
                self.session.thread_id,
                turn_id,
            ),
            store: self.session.session_store.clone(),
            history: self.session.history.clone(),
            initial_history: history.clone(),
            current_user: user_message.clone(),
            steering: steering_model.clone(),
            tools: workflow_context.tool_recorder.clone(),
            capture: Default::default(),
            recorded_compactions: Default::default(),
        });
        workflow_context.history_recorder = checkpoint_recorder.clone();
        workflow_context.tool_recorder = checkpoint_recorder;
        let workflow_timeout_ms = snapshot.runtime.registry.runtime_config.workflow_timeout_ms;
        let workflow =
            snapshot
                .runtime
                .registry
                .workflow
                .run(task.clone(), history.clone(), workflow_context);
        let workflow_result = if workflow_timeout_ms == 0 {
            workflow.await
        } else {
            match timeout(Duration::from_millis(workflow_timeout_ms), workflow).await {
                Ok(result) => result,
                Err(_) => {
                    cancellation.cancel();
                    Err(TurnAbort::WorkflowTimeout {
                        timeout_ms: workflow_timeout_ms,
                    }
                    .into())
                }
            }
        };
        let delivery_records = steering_model.delivery_records().await;
        let mut workflow_output = match workflow_result {
            Ok(output) => output,
            Err(error) => {
                return self
                    .fail_turn_with_progress(
                        turn_id,
                        error,
                        &history,
                        &user_message,
                        &delivery_records,
                    )
                    .await;
            }
        };
        if cancellation.is_cancelled() {
            return self
                .fail_turn_preserving_steering(
                    turn_id,
                    TurnAbort::Canceled.into(),
                    &delivery_records,
                )
                .await;
        }
        let runtime_user_messages =
            match weave_deliveries_into_output(&mut workflow_output, &delivery_records) {
                Ok(messages) => messages,
                Err(error) => {
                    return self
                        .fail_turn_preserving_steering(turn_id, error, &delivery_records)
                        .await;
                }
            };
        let history_update = match prepare_history_update(
            &history,
            &user_message,
            &workflow_output.new_messages,
            workflow_output.history_replacement.as_deref(),
            &workflow_output.compactions,
            &runtime_user_messages,
        ) {
            Ok(update) => update,
            Err(error) => {
                return self
                    .fail_turn_preserving_steering(turn_id, error, &delivery_records)
                    .await;
            }
        };
        self.commit_history_update(turn_id, history_update, &workflow_output.compactions, false)
            .await?;
        Ok(workflow_output.output)
    }

    async fn persist_current_user_message(
        &self,
        turn_id: crate::domain::TurnId,
        user_message: &CanonicalMessage,
        config_snapshot: Option<&SessionConfigSnapshot>,
    ) -> Result<Vec<CanonicalMessage>> {
        let mut history = self.session.history.lock().await;
        if let Some(session_store) = &self.session.session_store {
            session_store
                .append_history(
                    self.session.thread_id,
                    Some(turn_id),
                    std::slice::from_ref(user_message),
                )
                .await?;
            self.persist_config_snapshot_for_session(config_snapshot);
        }
        history.push(user_message.clone());
        Ok(history.clone())
    }
}

pub(crate) fn turn_settlement_status(
    error: &anyhow::Error,
    cancellation_is_set: bool,
) -> crate::core::TurnSettlementStatus {
    for cause in error.chain() {
        match cause.downcast_ref::<TurnAbort>() {
            Some(TurnAbort::WorkflowTimeout { .. }) => {
                return crate::core::TurnSettlementStatus::Timeout;
            }
            Some(TurnAbort::Canceled) => {
                return crate::core::TurnSettlementStatus::Canceled;
            }
            None => {}
        }
    }
    if cancellation_is_set {
        crate::core::TurnSettlementStatus::Canceled
    } else {
        crate::core::TurnSettlementStatus::Error
    }
}
