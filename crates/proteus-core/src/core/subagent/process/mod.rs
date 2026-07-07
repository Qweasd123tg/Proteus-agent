//! Builtin slot `subagent`, реализация `process`: ребёнок — отдельный
//! процесс `proteus server stdio` со своим named config («роль = профиль»).
//!
//! Родитель общается с ребёнком по stdio JSONL-протоколу app-server-а
//! (`StdioRequest`/`StdioOutput`): отправляет turn через `Send`, форвардит
//! approval/user-input запросы ребёнка в родительские transports (реальное
//! решение принимает пользователь родительской session), пере-эмитит
//! tool-события ребёнка под выделенным `child_thread_id` и возвращает
//! финальный `AgentOutput` как summary. Изоляция структурная: policy,
//! tools, model и permission mode ребёнка задаются его конфигом; сбой или
//! kill ребёнка не задевает родительский runtime (cancel = `Cancel` +
//! grace, затем kill).

mod child;
mod config;
#[cfg(test)]
mod tests;

use std::{collections::HashMap, path::PathBuf, sync::Mutex as StdMutex, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use proteus_contracts::app_protocol::{AppServerEvent, StdioOutput, StdioRequest};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout, timeout_at};

use crate::{
    contracts::{
        ApprovalRequest, RequestOrigin, RuntimeContext, SubagentRequest, SubagentResult,
        SubagentRoleSpec, SubagentRunner, SubagentStatus, UserInputResponse,
    },
    domain::{AgentOutput, Event, EventContext, ThreadId, new_call_id, new_thread_id},
    model_standard::TokenUsage,
};

use super::child_loop::{subagent_status_label, truncate_at_char_boundary};
use child::ChildProcess;
use config::{ProcessRoleConfig, ProcessSubagentConfig, build_process_role_specs};

/// Сколько ждать ответ ребёнка на служебные запросы (ClearHistory).
const CONTROL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ProcessSubagentRunner {
    specs: Vec<SubagentRoleSpec>,
    roles: HashMap<String, RoleState>,
    binary: PathBuf,
    max_depth: u64,
    cancel_grace: Duration,
    /// task_id → (role, generation ребёнка): resume валиден, пока жив тот же
    /// процесс (история живёт в session ребёнка).
    resumable: StdMutex<HashMap<String, ResumableProcessTask>>,
}

struct RoleState {
    config: ProcessRoleConfig,
    slot: tokio::sync::Mutex<RoleSlot>,
}

#[derive(Default)]
struct RoleSlot {
    child: Option<ChildProcess>,
    /// Инкрементируется на каждый (re)spawn: инвалидирует task_id-ы,
    /// выданные предыдущему процессу.
    generation: u64,
    /// Ребёнок уже отработал turn с момента spawn: свежая (не-resume)
    /// задача требует `ClearHistory` перед `Send`.
    used: bool,
}

#[derive(Debug, Clone)]
struct ResumableProcessTask {
    role: String,
    generation: u64,
}

impl ProcessSubagentRunner {
    /// Строит runner из значения `module_config.subagent.process`.
    pub fn from_config(config: Value) -> Result<Self> {
        let parsed: ProcessSubagentConfig = if config.is_null() {
            ProcessSubagentConfig::default()
        } else {
            serde_json::from_value(config)
                .context("failed to parse module_config.subagent.process")?
        };

        let specs = build_process_role_specs(&parsed.roles)?;
        let binary = match parsed.binary {
            Some(binary) => binary,
            None => std::env::current_exe()
                .context("failed to resolve current executable for subagent children")?,
        };
        let roles = parsed
            .roles
            .into_iter()
            .map(|role| {
                (
                    role.name.clone(),
                    RoleState {
                        config: role,
                        slot: tokio::sync::Mutex::new(RoleSlot::default()),
                    },
                )
            })
            .collect();

        Ok(Self {
            specs,
            roles,
            binary,
            max_depth: parsed.max_depth,
            cancel_grace: Duration::from_millis(parsed.cancel_grace_ms),
            resumable: StdMutex::new(HashMap::new()),
        })
    }

    fn resumable_task(&self, task_id: &str) -> Result<Option<ResumableProcessTask>> {
        Ok(self
            .resumable
            .lock()
            .map_err(|_| anyhow!("subagent resumable map lock poisoned"))?
            .get(task_id)
            .cloned())
    }

    fn save_resumable_task(&self, task_id: String, task: ResumableProcessTask) -> Result<()> {
        self.resumable
            .lock()
            .map_err(|_| anyhow!("subagent resumable map lock poisoned"))?
            .insert(task_id, task);
        Ok(())
    }
}

#[async_trait]
impl SubagentRunner for ProcessSubagentRunner {
    fn roles(&self) -> Vec<SubagentRoleSpec> {
        self.specs.clone()
    }

    async fn run(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentResult> {
        let spec = self
            .specs
            .iter()
            .find(|spec| spec.name == request.role)
            .cloned()
            .ok_or_else(|| anyhow!("unknown subagent role: {}", request.role))?;
        let role = self
            .roles
            .get(&request.role)
            .ok_or_else(|| anyhow!("unknown subagent role: {}", request.role))?;

        let depth = request
            .metadata
            .get("subagent_depth")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if depth >= self.max_depth {
            bail!(
                "subagent depth limit reached (depth {depth}, max_depth {})",
                self.max_depth
            );
        }

        let mut slot = role.slot.lock().await;
        if slot.child.as_mut().is_some_and(|child| !child.is_alive()) {
            slot.child = None;
        }
        if slot.child.is_none() {
            slot.child = Some(ChildProcess::spawn(
                &self.binary,
                &role.config.config,
                &role.config.args,
                &request.task.cwd,
            )?);
            slot.generation = slot.generation.wrapping_add(1);
            slot.used = false;
        }

        let resume_task_id = request.metadata.get("task_id").and_then(Value::as_str);
        let (child_thread_id, is_resume) = match resume_task_id {
            Some(task_id) => {
                let entry = self
                    .resumable_task(task_id)?
                    .ok_or_else(|| anyhow!("unknown task_id (expired or from another session)"))?;
                if entry.role != request.role {
                    bail!(
                        "task_id belongs to subagent role {}, but request role is {}",
                        entry.role,
                        request.role
                    );
                }
                if entry.generation != slot.generation {
                    bail!("unknown task_id (subagent child process was restarted)");
                }
                let child_thread_id = task_id.parse::<ThreadId>().with_context(|| {
                    format!("invalid task_id for resumable subagent: {task_id}")
                })?;
                (child_thread_id, true)
            }
            None => (new_thread_id(), false),
        };

        let generation = slot.generation;
        let needs_clear = !is_resume && slot.used;
        let slot_ref = &mut *slot;
        let child = slot_ref
            .child
            .as_mut()
            .expect("child process spawned above");
        child.drain_stale_outputs();

        if needs_clear {
            clear_child_history(child).await?;
        }

        ctx.emit(Event::SubagentStarted {
            role: spec.name.clone(),
            description: request.description.clone(),
            child_thread_id,
        })
        .await?;

        let text = if !is_resume && !spec.prompt.trim().is_empty() {
            format!("{}\n\n{}", spec.prompt, request.prompt)
        } else {
            request.prompt.clone()
        };
        let forwarder = ChildEventForwarder {
            ctx: &ctx,
            child_thread_id,
            role: spec.name.clone(),
        };
        let mut tracker = TurnTracker::default();

        let send_id = new_call_id();
        let body_result: Result<TurnEnd> = async {
            child
                .send(&StdioRequest::Send {
                    id: Some(send_id.clone()),
                    text,
                })
                .await?;

            match spec.limits.timeout_ms {
                Some(timeout_ms) => {
                    match timeout(
                        Duration::from_millis(timeout_ms),
                        drive_turn(child, &forwarder, &send_id, &mut tracker, self.cancel_grace),
                    )
                    .await
                    {
                        Ok(end) => end,
                        Err(_elapsed) => {
                            let clean = cancel_child_turn(
                                child,
                                &forwarder,
                                &send_id,
                                &mut tracker,
                                self.cancel_grace,
                            )
                            .await;
                            if !clean {
                                child.kill().await;
                            }
                            Ok(TurnEnd::Interrupted(SubagentStatus::TimedOut))
                        }
                    }
                }
                None => {
                    drive_turn(child, &forwarder, &send_id, &mut tracker, self.cancel_grace).await
                }
            }
        }
        .await;

        match body_result {
            Ok(end) => {
                slot_ref.used = true;
                let child_alive = slot_ref
                    .child
                    .as_mut()
                    .is_some_and(|child| child.is_alive());
                if !child_alive {
                    slot_ref.child = None;
                }

                let (status, summary) = match end {
                    TurnEnd::Completed(text) => (SubagentStatus::Completed, text),
                    TurnEnd::Interrupted(status) => (status, tracker.partial_text()),
                };
                let mut summary = summary;
                if let Some(max_bytes) = spec.limits.max_summary_bytes {
                    summary = truncate_at_char_boundary(summary, max_bytes);
                }

                let resumable = child_alive;
                if resumable {
                    self.save_resumable_task(
                        child_thread_id.to_string(),
                        ResumableProcessTask {
                            role: spec.name.clone(),
                            generation,
                        },
                    )?;
                }

                ctx.emit(Event::SubagentFinished {
                    role: spec.name.clone(),
                    status: subagent_status_label(status),
                    iterations: tracker.iterations,
                    child_thread_id,
                })
                .await?;

                let mut result = SubagentResult::new(summary, status, tracker.iterations)
                    .with_child_thread_id(child_thread_id)
                    .with_metadata(json!({ "resumable": resumable }));
                if let Some(usage) = tracker.usage.clone() {
                    result = result.with_usage(usage);
                }
                Ok(result)
            }
            Err(error) => {
                // Fatal-путь (EOF, ошибка child turn-а): процесс считается
                // невалидным — следующий run пересоздаст его.
                if let Some(child) = slot_ref.child.as_mut() {
                    child.kill().await;
                }
                slot_ref.child = None;
                let _ = ctx
                    .emit(Event::SubagentFinished {
                        role: spec.name.clone(),
                        status: "errored".into(),
                        iterations: tracker.iterations,
                        child_thread_id,
                    })
                    .await;
                Err(error)
            }
        }
    }
}

/// Терминальный исход turn-а ребёнка (fatal-ошибки идут через `Err`).
enum TurnEnd {
    Completed(String),
    Interrupted(SubagentStatus),
}

#[derive(Default)]
struct TurnTracker {
    iterations: u32,
    usage: Option<TokenUsage>,
    /// Текст завершённых model-ответов и хвост текущего стрима — единственный
    /// источник partial summary при cancel/timeout.
    last_text: Option<String>,
    stream_buffer: String,
    cancel_sent: bool,
}

impl TurnTracker {
    fn observe(&mut self, event: &Event) {
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
                    super::child_loop::accumulate_usage(&mut self.usage, Some(actual));
                }
            }
            _ => {}
        }
    }

    fn partial_text(&self) -> String {
        if !self.stream_buffer.trim().is_empty() {
            self.stream_buffer.clone()
        } else {
            self.last_text.clone().unwrap_or_default()
        }
    }
}

/// Пере-эмиссия событий ребёнка в родительский event stream под
/// `child_thread_id` и форвардинг интерактивных запросов.
struct ChildEventForwarder<'a> {
    ctx: &'a RuntimeContext,
    child_thread_id: ThreadId,
    role: String,
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

    /// Форвардит approval ребёнка в родительский transport и возвращает
    /// reply для ребёнка. `None` — родительский turn отменён во время
    /// ожидания решения.
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

    /// Форвардит typed user-input запрос ребёнка. `None` — родительский
    /// turn отменён.
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

/// События ребёнка, которые пере-эмитятся в родительский stream. Набор
/// согласован с sequential runner-ом: там под child thread идут tool-события
/// orchestrator-а; модельная телеметрия, deltas и session/turn lifecycle
/// ребёнка остаются в его собственном event log.
fn should_forward_child_event(event: &Event) -> bool {
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

/// Главный цикл turn-а: читает outputs ребёнка до Response на наш Send,
/// форвардя события и интерактивные запросы. Отмена родителя запускает
/// cancel-протокол (`Cancel` → grace-дожидание Response → kill).
async fn drive_turn(
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
    let clean = cancel_child_turn(child, forwarder, send_id, tracker, cancel_grace).await;
    if !clean {
        child.kill().await;
    }
    Ok(TurnEnd::Interrupted(SubagentStatus::Cancelled))
}

enum OutputVerdict {
    Continue,
    Finished(TurnEnd),
    /// Родительский turn отменён во время ожидания approval/user-input.
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

/// Отправляет ребёнку Cancel и дожидается терминального Response в пределах
/// grace-окна, продолжая форвардить события. `true` — turn ребёнка завершился
/// штатно (процесс можно переиспользовать), `false` — ребёнок не ответил.
async fn cancel_child_turn(
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

/// Сбрасывает историю ребёнка перед свежей (не-resume) задачей: процесс
/// переиспользуется, но каждый новый task начинается с чистой session-истории.
async fn clear_child_history(child: &mut ChildProcess) -> Result<()> {
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
