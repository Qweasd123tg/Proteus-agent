//! Session-owned admission, cancellation and settlement shared by HTTP/stdio.
use super::{AppServerEvent, AppServerHandle, completed_transcript};
use crate::{
    contracts::CancellationToken,
    core::{SteeringQueueReceipt, TurnSettlementStatus, UserMessageReservation},
    domain::AgentOutput,
};
use anyhow::{Result, anyhow};
use proteus_contracts::app_protocol::{AppExecutionState, AppRun, AppRunStatus};
use tokio::sync::{oneshot, watch};

#[derive(Default)]
pub(super) struct RunRegistry {
    active: Option<RunningRun>,
    last: Option<AppRun>,
    closed: bool,
    command_active: bool,
}

pub(super) struct RunningRun {
    pub info: AppRun,
    pub cancellation: CancellationToken,
    settled: watch::Receiver<bool>,
}

pub(super) enum SendDispatch {
    Started(oneshot::Receiver<Result<AgentOutput>>),
    Queued(SteeringQueueReceipt),
}

impl RunRegistry {
    fn snapshot(&self) -> AppExecutionState {
        AppExecutionState {
            active: self.active.as_ref().map(|run| run.info.clone()),
            last: self.last.clone(),
        }
    }
    fn publish(&self, server: &AppServerHandle) {
        let _ = server.events.send(AppServerEvent::ExecutionUpdated {
            execution: self.snapshot(),
        });
    }
}

impl AppServerHandle {
    pub async fn clear_history(&self) -> Result<()> {
        let runs = self.runs.lock().await;
        if runs.closed {
            return Err(anyhow!("session is shutting down"));
        }
        if runs.active.is_some() {
            return Err(anyhow!("cannot clear history while a run is active"));
        }
        self.runtime.clear_history().await?;
        // A successful Replace([]) clears the canonical journal transcript.
        self.events.finish_progress(Ok(Vec::new()));
        self.events.publish_snapshot()
    }

    pub(super) async fn dispatch_user_message(
        &self,
        run_id: Option<String>,
        input: impl Into<crate::domain::UserMessageInput>,
        options: crate::domain::RunOptions,
        cancellation: CancellationToken,
    ) -> Result<SendDispatch> {
        self.admit_user_message(run_id, input, options, cancellation, true)
            .await
    }

    pub(super) async fn admit_user_message(
        &self,
        run_id: Option<String>,
        input: impl Into<crate::domain::UserMessageInput>,
        options: crate::domain::RunOptions,
        cancellation: CancellationToken,
        allow_queue: bool,
    ) -> Result<SendDispatch> {
        let run_id = run_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut runs = self.runs.lock().await;
        if runs.closed {
            return Err(anyhow!("session is shutting down"));
        }
        if runs.command_active {
            return Err(anyhow!("session has an active command"));
        }
        if !allow_queue && runs.active.is_some() {
            return Err(anyhow!("session already has an active run"));
        }
        if runs
            .active
            .as_ref()
            .is_some_and(|r| r.info.run_id == run_id)
        {
            return Err(anyhow!("run id is already active: {run_id}"));
        }
        let reserved = match self.reserve_user_message(input, options).await? {
            UserMessageReservation::Queued(receipt) => return Ok(SendDispatch::Queued(receipt)),
            UserMessageReservation::Start(reserved) => reserved,
        };
        let options = reserved.options();
        let (settled_tx, settled) = watch::channel(false);
        runs.active = Some(RunningRun {
            info: AppRun {
                run_id: run_id.clone(),
                options: options.clone(),
                status: AppRunStatus::Running,
                error: None,
            },
            cancellation: cancellation.clone(),
            settled,
        });
        runs.publish(self);
        let (tx, rx) = oneshot::channel();
        let server = self.clone();
        tokio::spawn(async move {
            // Keep the runtime reservation until its terminal state has been
            // published under the admission lock. A new send cannot overtake it.
            let completion = server
                .runtime
                .run_reserved_completion(reserved, cancellation.clone())
                .await;
            // The reservation remains held until the registry publishes the
            // terminal state, so no next turn can overtake this projection.
            let history = completed_transcript(server.runtime.clone()).await;
            let mut runs = server.runs.lock().await;
            server.events.finish_progress(history);
            let result = server.publish_turn_completion(completion);
            let (status, error) = match &result {
                Ok(_) => (AppRunStatus::Success, None),
                Err(error) => {
                    let status = match crate::core::turn_settlement_status(
                        error,
                        cancellation.is_cancelled(),
                    ) {
                        TurnSettlementStatus::Canceled => AppRunStatus::Canceled,
                        TurnSettlementStatus::Timeout => AppRunStatus::Timeout,
                        _ => AppRunStatus::Error,
                    };
                    (status, Some(format!("{error:#}")))
                }
            };
            runs.active = None;
            runs.last = Some(AppRun {
                run_id,
                options,
                status,
                error,
            });
            runs.publish(&server);
            let result = server.events.publish_snapshot().and(result);
            drop(runs);
            settled_tx.send_replace(true);
            let _ = tx.send(result);
        });
        Ok(SendDispatch::Started(rx))
    }

    pub(super) async fn cancel_run(&self, target_id: &str) -> Result<()> {
        let mut runs = self.runs.lock().await;
        let run = runs
            .active
            .as_mut()
            .filter(|r| r.info.run_id == target_id)
            .ok_or_else(|| anyhow!("unknown or completed run id for session: {target_id}"))?;
        run.info.status = AppRunStatus::CancelRequested;
        run.cancellation.cancel();
        runs.publish(self);
        Ok(())
    }

    pub(super) async fn run_module_command(
        &self,
        id: Option<String>,
        name: String,
        args: String,
    ) -> Result<crate::domain::ToolResult> {
        let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let cancellation = CancellationToken::new();
        let (settled_tx, settled) = watch::channel(false);
        let mut runs = self.runs.lock().await;
        anyhow::ensure!(
            !runs.closed && runs.active.is_none(),
            "commands require an idle session"
        );
        runs.command_active = true;
        runs.active = Some(RunningRun {
            info: AppRun {
                run_id: id.clone(),
                options: Default::default(),
                status: AppRunStatus::Running,
                error: None,
            },
            cancellation: cancellation.clone(),
            settled,
        });
        runs.publish(self);
        drop(runs);
        // Independent of the response consumer: disconnect cannot abandon admission.
        let server = self.clone();
        let task = tokio::spawn(async move {
            let result = server
                .runtime
                .execute_user_command(&name, &args, cancellation.clone())
                .await;
            let mut runs = server.runs.lock().await;
            runs.active = None;
            runs.command_active = false;
            runs.last = Some(AppRun {
                run_id: id,
                options: Default::default(),
                status: if cancellation.is_cancelled() {
                    AppRunStatus::Canceled
                } else if result.as_ref().is_ok_and(|r| r.ok) {
                    AppRunStatus::Success
                } else {
                    AppRunStatus::Error
                },
                error: result.as_ref().err().map(|e| format!("{e:#}")),
            });
            runs.publish(&server);
            drop(runs);
            settled_tx.send_replace(true);
            result
        });
        task.await?
    }

    pub(super) async fn close_runs(&self) {
        let settled = {
            let mut runs = self.runs.lock().await;
            runs.closed = true;
            let settled = runs.active.as_mut().map(|run| {
                run.info.status = AppRunStatus::CancelRequested;
                run.cancellation.cancel();
                run.settled.clone()
            });
            runs.publish(self);
            settled
        };
        if let Some(mut settled) = settled {
            // Wait for the runtime task, not the HTTP/stdout response consumer.
            // A deleted session must not be recreated by late settlement writes.
            let _ = settled.wait_for(|done| *done).await;
        }
    }

    pub(super) async fn running_run_ids(&self) -> Vec<String> {
        self.runs
            .lock()
            .await
            .active
            .as_ref()
            .map(|r| vec![r.info.run_id.clone()])
            .unwrap_or_default()
    }
}

#[cfg(test)]
impl AppServerHandle {
    pub(super) async fn register_test_run(&self, id: &str, cancellation: CancellationToken) {
        let mut runs = self.runs.lock().await;
        runs.active = Some(RunningRun {
            info: AppRun {
                run_id: id.to_owned(),
                options: Default::default(),
                status: AppRunStatus::Running,
                error: None,
            },
            cancellation,
            settled: watch::channel(true).1,
        });
        runs.publish(self);
    }
}
