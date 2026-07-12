//! One stdio turn on a leased process child: event forwarding, interactive
//! request bridging, usage tracking and cancel/timeout protocol.

use std::time::Duration;

use anyhow::{Result, bail};
use proteus_contracts::app_protocol::{AppServerEvent, StdioOutput, StdioRequest};
use tokio::time::{Instant, timeout_at};

use super::child::ChildProcess;
use crate::{
    contracts::{
        ApprovalRequest, BudgetTracker, RequestOrigin, RuntimeContext, SubagentStatus,
        UserInputResponse,
    },
    domain::{AgentOutput, Event, EventContext, ThreadId, new_call_id},
    model_standard::TokenUsage,
};

/// Сколько ждать ответ ребёнка на служебные запросы (`ClearHistory`).
const CONTROL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// Терминальный исход turn-а ребёнка (fatal-ошибки идут через `Err`).
pub(super) enum TurnEnd {
    Completed(String),
    Interrupted(SubagentStatus),
}

#[derive(Default)]
pub(super) struct TurnTracker {
    pub(super) iterations: u32,
    pub(super) usage: Option<TokenUsage>,
    /// Текст завершённых model-ответов и хвост текущего стрима — единственный
    /// источник partial summary при cancel/timeout.
    last_text: Option<String>,
    stream_buffer: String,
    cancel_sent: bool,
    /// Token-бюджет запуска (`SubagentLimits::max_total_tokens`). Скоупится
    /// на запуск, не на task_id: resume получает новое окно.
    pub(super) budget: BudgetTracker,
}

impl TurnTracker {
    pub(super) fn with_budget(max_total_tokens: Option<u64>) -> Self {
        Self {
            budget: BudgetTracker::new(max_total_tokens),
            ..Self::default()
        }
    }

    pub(super) fn observe(&mut self, event: &Event) {
        match event {
            Event::ModelRequestPrepared { .. } => {
                self.iterations = self.iterations.saturating_add(1);
                self.stream_buffer.clear();
            }
            Event::AssistantTextDelta { text } => {
                self.stream_buffer.push_str(text);
            }
            Event::ModelResponseReceived { .. } => {
                if !self.stream_buffer.trim().is_empty() {
                    self.last_text = Some(std::mem::take(&mut self.stream_buffer));
                } else {
                    self.stream_buffer.clear();
                }
            }
            Event::TokenUsageUpdated { usage } => {
                if let Some(actual) = usage.actual.as_ref() {
                    super::super::child_loop::accumulate_usage(&mut self.usage, Some(actual));
                    self.budget.record(Some(actual));
                }
            }
            _ => {}
        }
    }

    pub(super) fn partial_text(&self) -> String {
        if !self.stream_buffer.trim().is_empty() {
            self.stream_buffer.clone()
        } else {
            self.last_text.clone().unwrap_or_default()
        }
    }
}

/// Пере-эмиссия событий ребёнка в родительский event stream под
/// `child_thread_id` и форвардинг интерактивных запросов.
pub(super) struct ChildEventForwarder<'a> {
    pub(super) ctx: &'a RuntimeContext,
    pub(super) child_thread_id: ThreadId,
    pub(super) role: String,
}

impl ChildEventForwarder<'_> {
    fn event_context(&self) -> EventContext {
        EventContext::new(
            self.ctx.session_id,
            self.child_thread_id,
            Some(self.ctx.turn_id),
        )
    }

    fn origin(&self) -> RequestOrigin {
        RequestOrigin::new(self.child_thread_id, self.ctx.turn_id).with_label(self.role.clone())
    }

    async fn forward_runtime_event(&self, event: Event) {
        if !should_forward_child_event(&event) {
            return;
        }
        let _ = self.ctx.events.emit(self.event_context(), event).await;
    }

    async fn forward_approval(
        &self,
        request: proteus_contracts::app_protocol::AppApprovalRequest,
    ) -> Option<StdioRequest> {
        let approval_id = request.approval_id.clone();
        let forwarded = ApprovalRequest::new(
            request.call.clone(),
            request.cwd.clone(),
            request.reason.clone(),
            request.tool_spec.clone(),
        )
        .with_origin(self.origin());
        let response = tokio::select! {
            response = self.ctx.approval.request_approval(forwarded) => response,
            _ = self.ctx.cancellation.cancelled() => return None,
        };
        let (approved, note, cache) = match response {
            Ok(response) => (response.approved, response.note, response.cache),
            Err(error) => (
                false,
                Some(format!("parent approval transport failed: {error:#}")),
                Default::default(),
            ),
        };
        Some(StdioRequest::Approval {
            id: None,
            approval_id,
            approved,
            note,
            cache,
        })
    }

    async fn forward_user_input(
        &self,
        request: crate::contracts::UserInputRequest,
    ) -> Option<StdioRequest> {
        let request_id = request.request_id.clone();
        let forwarded = request.with_origin(self.origin());
        let response = tokio::select! {
            response = self.ctx.user_input.request_user_input(forwarded) => response,
            _ = self.ctx.cancellation.cancelled() => return None,
        };
        let response = response.unwrap_or_else(|_| UserInputResponse::empty());
        Some(StdioRequest::UserInput {
            id: None,
            request_id,
            response,
        })
    }
}

pub(super) fn should_forward_child_event(event: &Event) -> bool {
    matches!(
        event,
        Event::ToolCallRequested { .. }
            | Event::ApprovalRequested { .. }
            | Event::ApprovalResolved { .. }
            | Event::ToolFinished { .. }
            | Event::PatchApplied { .. }
            | Event::MemoryWritten { .. }
            | Event::SubagentStarted { .. }
            | Event::SubagentFinished { .. }
            | Event::Error { .. }
    )
}

pub(super) async fn drive_turn(
    child: &mut ChildProcess,
    forwarder: &ChildEventForwarder<'_>,
    send_id: &str,
    tracker: &mut TurnTracker,
    cancel_grace: Duration,
) -> Result<TurnEnd> {
    loop {
        if forwarder.ctx.cancellation.is_cancelled() && !tracker.cancel_sent {
            return finish_cancelled(child, forwarder, send_id, tracker, cancel_grace).await;
        }
        if tracker.budget.exceeded() && !tracker.cancel_sent {
            return finish_interrupted_with(
                child,
                forwarder,
                send_id,
                tracker,
                cancel_grace,
                SubagentStatus::TokenBudgetExceeded,
            )
            .await;
        }

        let output = tokio::select! {
            output = child.next_output() => output,
            _ = forwarder.ctx.cancellation.cancelled() => {
                return finish_cancelled(child, forwarder, send_id, tracker, cancel_grace).await;
            }
        };
        let Some(output) = output else {
            bail!("subagent child process exited unexpectedly");
        };

        match handle_output(child, forwarder, send_id, tracker, output).await? {
            OutputVerdict::Continue => {}
            OutputVerdict::Finished(end) => return Ok(end),
            OutputVerdict::CancelRequested => {
                return finish_cancelled(child, forwarder, send_id, tracker, cancel_grace).await;
            }
        }
    }
}

async fn finish_cancelled(
    child: &mut ChildProcess,
    forwarder: &ChildEventForwarder<'_>,
    send_id: &str,
    tracker: &mut TurnTracker,
    cancel_grace: Duration,
) -> Result<TurnEnd> {
    finish_interrupted_with(
        child,
        forwarder,
        send_id,
        tracker,
        cancel_grace,
        SubagentStatus::Cancelled,
    )
    .await
}

async fn finish_interrupted_with(
    child: &mut ChildProcess,
    forwarder: &ChildEventForwarder<'_>,
    send_id: &str,
    tracker: &mut TurnTracker,
    cancel_grace: Duration,
    status: SubagentStatus,
) -> Result<TurnEnd> {
    let clean = cancel_child_turn(child, forwarder, send_id, tracker, cancel_grace).await;
    if !clean {
        child.kill().await;
    }
    Ok(TurnEnd::Interrupted(status))
}

enum OutputVerdict {
    Continue,
    Finished(TurnEnd),
    CancelRequested,
}

async fn handle_output(
    child: &mut ChildProcess,
    forwarder: &ChildEventForwarder<'_>,
    send_id: &str,
    tracker: &mut TurnTracker,
    output: StdioOutput,
) -> Result<OutputVerdict> {
    match output {
        StdioOutput::Event { event } => match *event {
            AppServerEvent::Runtime { envelope } => {
                tracker.observe(&envelope.event);
                forwarder.forward_runtime_event(envelope.event).await;
                Ok(OutputVerdict::Continue)
            }
            AppServerEvent::ApprovalRequested { request } => {
                match forwarder.forward_approval(*request).await {
                    Some(reply) => {
                        child.send(&reply).await?;
                        Ok(OutputVerdict::Continue)
                    }
                    None => Ok(OutputVerdict::CancelRequested),
                }
            }
            AppServerEvent::UserInputRequested { request } => {
                match forwarder.forward_user_input(*request).await {
                    Some(reply) => {
                        child.send(&reply).await?;
                        Ok(OutputVerdict::Continue)
                    }
                    None => Ok(OutputVerdict::CancelRequested),
                }
            }
            AppServerEvent::Shutdown => bail!("subagent child app-server shut down mid-turn"),
            _ => Ok(OutputVerdict::Continue),
        },
        StdioOutput::Response { id, ok, output, .. } if id.as_deref() == Some(send_id) && ok => {
            let text = output
                .and_then(|value| serde_json::from_value::<AgentOutput>(value).ok())
                .map(|output| output.text)
                .unwrap_or_default();
            Ok(OutputVerdict::Finished(TurnEnd::Completed(text)))
        }
        StdioOutput::Response { id, error, .. } if id.as_deref() == Some(send_id) => {
            if tracker.cancel_sent {
                Ok(OutputVerdict::Finished(TurnEnd::Interrupted(
                    SubagentStatus::Cancelled,
                )))
            } else {
                bail!(
                    "subagent child turn failed: {}",
                    error.unwrap_or_else(|| "unknown error".to_owned())
                );
            }
        }
        StdioOutput::Response { .. } => Ok(OutputVerdict::Continue),
        _ => Ok(OutputVerdict::Continue),
    }
}

pub(super) async fn cancel_child_turn(
    child: &mut ChildProcess,
    forwarder: &ChildEventForwarder<'_>,
    send_id: &str,
    tracker: &mut TurnTracker,
    cancel_grace: Duration,
) -> bool {
    if !tracker.cancel_sent {
        tracker.cancel_sent = true;
        let cancel = StdioRequest::Cancel {
            id: None,
            target_id: send_id.to_owned(),
        };
        if child.send(&cancel).await.is_err() {
            return false;
        }
    }

    let deadline = Instant::now() + cancel_grace;
    loop {
        let output = match timeout_at(deadline, child.next_output()).await {
            Ok(Some(output)) => output,
            Ok(None) | Err(_) => return false,
        };
        match output {
            StdioOutput::Response { id, .. } if id.as_deref() == Some(send_id) => return true,
            StdioOutput::Event { event } => {
                if let AppServerEvent::Runtime { envelope } = *event {
                    tracker.observe(&envelope.event);
                    forwarder.forward_runtime_event(envelope.event).await;
                }
            }
            _ => {}
        }
    }
}

pub(super) async fn clear_child_history(child: &mut ChildProcess) -> Result<()> {
    let request_id = new_call_id();
    child
        .send(&StdioRequest::ClearHistory {
            id: Some(request_id.clone()),
        })
        .await?;
    let deadline = Instant::now() + CONTROL_RESPONSE_TIMEOUT;
    loop {
        match timeout_at(deadline, child.next_output()).await {
            Ok(Some(StdioOutput::Response { id, ok, error, .. }))
                if id.as_deref() == Some(request_id.as_str()) =>
            {
                if !ok {
                    bail!(
                        "subagent child failed to clear history: {}",
                        error.unwrap_or_else(|| "unknown error".to_owned())
                    );
                }
                return Ok(());
            }
            Ok(Some(_)) => {}
            Ok(None) => bail!("subagent child process exited while clearing history"),
            Err(_) => bail!("subagent child did not confirm history clear in time"),
        }
    }
}
