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
//!
//! Исполнение — через `spawn`/`wait`/`cancel` (см. `PendingChildren`):
//! каждый запуск живёт detached tokio-таской на child-токене отмены.
//! Процессы роли переиспользуются через пул: до `max_processes`
//! одновременных детей на роль (semaphore), свободные процессы ждут в
//! `idle` и получают `ClearHistory` перед свежей задачей. `run` =
//! `spawn` + `wait`.

mod child;
mod config;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use proteus_contracts::app_protocol::{AppServerEvent, StdioOutput, StdioRequest};
use serde_json::{Value, json};
use tokio::{
    sync::Semaphore,
    time::{Instant, timeout, timeout_at},
};

use crate::{
    contracts::{
        ApprovalRequest, RequestOrigin, RuntimeContext, SubagentHandle, SubagentRequest,
        SubagentResult, SubagentRoleSpec, SubagentRunner, SubagentStatus, UserInputResponse,
    },
    domain::{AgentOutput, Event, EventContext, ThreadId, new_call_id, new_thread_id},
    model_standard::TokenUsage,
};

use super::{
    child_context,
    child_loop::{subagent_status_label, truncate_at_char_boundary},
    pending::PendingChildren,
};
use child::ChildProcess;
use config::{ProcessRoleConfig, ProcessSubagentConfig, build_process_role_specs};

/// Сколько ждать ответ ребёнка на служебные запросы (ClearHistory).
const CONTROL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ProcessSubagentRunner {
    inner: Arc<RunnerInner>,
}

struct RunnerInner {
    specs: Vec<SubagentRoleSpec>,
    roles: HashMap<String, RoleState>,
    binary: PathBuf,
    max_depth: u64,
    max_parallel: usize,
    cancel_grace: Duration,
    /// task_id → (role, process id): resume валиден, пока жив тот же процесс
    /// и его session-история не была очищена под свежую задачу.
    resumable: StdMutex<HashMap<String, ResumableProcessTask>>,
    pending: StdMutex<PendingChildren>,
    next_process_id: AtomicU64,
}

struct RoleState {
    config: ProcessRoleConfig,
    /// Конкурентное владение процессами роли: до `max_processes` детей
    /// одновременно; остальные ждут свободного permit-а.
    permits: Arc<Semaphore>,
    pool: StdMutex<RolePool>,
}

#[derive(Default)]
struct RolePool {
    /// Свободные (возвращённые) процессы роли.
    idle: Vec<PooledChild>,
    /// process id-ы, находящиеся в аренде у запущенных детей.
    leased: HashSet<u64>,
}

struct PooledChild {
    id: u64,
    child: ChildProcess,
    /// Ребёнок уже отработал turn: свежая (не-resume) задача требует
    /// `ClearHistory` перед `Send`.
    used: bool,
}

#[derive(Debug, Clone)]
struct ResumableProcessTask {
    role: String,
    process_id: u64,
}

/// Результат подготовки запуска (до `SubagentStarted`): спека роли, thread
/// ребёнка и resume-цель, если задан валидный `task_id`.
struct PreparedProcess {
    spec: SubagentRoleSpec,
    child_thread_id: ThreadId,
    resume_process_id: Option<u64>,
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
                let max_processes = role.effective_max_processes();
                (
                    role.name.clone(),
                    RoleState {
                        config: role,
                        permits: Arc::new(Semaphore::new(max_processes)),
                        pool: StdMutex::new(RolePool::default()),
                    },
                )
            })
            .collect();

        Ok(Self {
            inner: Arc::new(RunnerInner {
                specs,
                roles,
                binary,
                max_depth: parsed.max_depth,
                max_parallel: parsed.max_parallel,
                cancel_grace: Duration::from_millis(parsed.cancel_grace_ms),
                resumable: StdMutex::new(HashMap::new()),
                pending: StdMutex::new(PendingChildren::default()),
                next_process_id: AtomicU64::new(0),
            }),
        })
    }
}

impl RunnerInner {
    fn lock_pending(&self) -> Result<MutexGuard<'_, PendingChildren>> {
        self.pending
            .lock()
            .map_err(|_| anyhow!("subagent pending registry lock poisoned"))
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

    /// Хоронит все task_id-ы, указывающие на процесс: вызывается, когда
    /// процесс умер или его session-история очищена под свежую задачу
    /// (resume на очищенной истории молча продолжил бы пустую session).
    fn purge_resumable_for_process(&self, process_id: u64) {
        if let Ok(mut map) = self.resumable.lock() {
            map.retain(|_, task| task.process_id != process_id);
        }
    }

    /// Резолвит роль, глубину и resume-цель до запуска: все ошибки
    /// подготовки возвращаются из `spawn` напрямую, ещё до
    /// `SubagentStarted`. Живость resume-процесса проверяется позже, при
    /// аренде из пула.
    fn prepare(&self, request: &SubagentRequest) -> Result<PreparedProcess> {
        let spec = self
            .specs
            .iter()
            .find(|spec| spec.name == request.role)
            .cloned()
            .ok_or_else(|| anyhow!("unknown subagent role: {}", request.role))?;
        if !self.roles.contains_key(&request.role) {
            bail!("unknown subagent role: {}", request.role);
        }

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

        let (child_thread_id, resume_process_id) =
            match request.metadata.get("task_id").and_then(Value::as_str) {
                Some(task_id) => {
                    let entry = self.resumable_task(task_id)?.ok_or_else(|| {
                        anyhow!("unknown task_id (expired or from another session)")
                    })?;
                    if entry.role != request.role {
                        bail!(
                            "task_id belongs to subagent role {}, but request role is {}",
                            entry.role,
                            request.role
                        );
                    }
                    let child_thread_id = task_id.parse::<ThreadId>().with_context(|| {
                        format!("invalid task_id for resumable subagent: {task_id}")
                    })?;
                    (child_thread_id, Some(entry.process_id))
                }
                None => (new_thread_id(), None),
            };

        Ok(PreparedProcess {
            spec,
            child_thread_id,
            resume_process_id,
        })
    }

    /// Берёт процесс роли в аренду. Resume — строго конкретный процесс по
    /// id; fresh — любой свободный живой процесс либо новый (permit уже
    /// ограничил количество одновременных аренд).
    fn lease_process(
        &self,
        role: &RoleState,
        resume_process_id: Option<u64>,
        cwd: &Path,
    ) -> Result<PooledChild> {
        let mut pool = role
            .pool
            .lock()
            .map_err(|_| anyhow!("subagent role pool lock poisoned"))?;
        match resume_process_id {
            Some(process_id) => {
                if let Some(index) = pool.idle.iter().position(|entry| entry.id == process_id) {
                    let mut entry = pool.idle.swap_remove(index);
                    if !entry.child.is_alive() {
                        self.purge_resumable_for_process(process_id);
                        bail!("unknown task_id (subagent child process exited)");
                    }
                    pool.leased.insert(process_id);
                    Ok(entry)
                } else if pool.leased.contains(&process_id) {
                    bail!("task_id belongs to a subagent that is still running; wait for it first")
                } else {
                    bail!("unknown task_id (subagent child process was restarted)")
                }
            }
            None => {
                while let Some(mut candidate) = pool.idle.pop() {
                    if candidate.child.is_alive() {
                        pool.leased.insert(candidate.id);
                        return Ok(candidate);
                    }
                    self.purge_resumable_for_process(candidate.id);
                }
                let id = self.next_process_id.fetch_add(1, Ordering::Relaxed);
                let child =
                    ChildProcess::spawn(&self.binary, &role.config.config, &role.config.args, cwd)?;
                pool.leased.insert(id);
                Ok(PooledChild {
                    id,
                    child,
                    used: false,
                })
            }
        }
    }

    /// Возвращает процесс в пул (живой — в `idle`, мёртвый — хоронится
    /// вместе со своими task_id-ами).
    fn release_process(&self, role: &RoleState, leased: PooledChild, alive: bool) {
        let Ok(mut pool) = role.pool.lock() else {
            return;
        };
        pool.leased.remove(&leased.id);
        if alive {
            pool.idle.push(leased);
        } else {
            self.purge_resumable_for_process(leased.id);
        }
    }

    /// Turn на арендованном процессе: опциональный ClearHistory, `Send`,
    /// затем drive до терминального Response с уважением role-таймаута.
    async fn run_leased_turn(
        &self,
        spec: &SubagentRoleSpec,
        request: &SubagentRequest,
        forwarder: &ChildEventForwarder<'_>,
        tracker: &mut TurnTracker,
        leased: &mut PooledChild,
        is_resume: bool,
    ) -> Result<TurnEnd> {
        leased.child.drain_stale_outputs();
        if !is_resume && leased.used {
            clear_child_history(&mut leased.child).await?;
            // История процесса очищена — прежние task_id-ы этого процесса
            // мертвы, resume по ним продолжил бы пустую session.
            self.purge_resumable_for_process(leased.id);
        }

        let text = if !is_resume && !spec.prompt.trim().is_empty() {
            format!("{}\n\n{}", spec.prompt, request.prompt)
        } else {
            request.prompt.clone()
        };
        let send_id = new_call_id();
        leased
            .child
            .send(&StdioRequest::Send {
                id: Some(send_id.clone()),
                text,
            })
            .await?;

        match spec.limits.timeout_ms {
            Some(timeout_ms) => {
                match timeout(
                    Duration::from_millis(timeout_ms),
                    drive_turn(
                        &mut leased.child,
                        forwarder,
                        &send_id,
                        tracker,
                        self.cancel_grace,
                    ),
                )
                .await
                {
                    Ok(end) => end,
                    Err(_elapsed) => {
                        let clean = cancel_child_turn(
                            &mut leased.child,
                            forwarder,
                            &send_id,
                            tracker,
                            self.cancel_grace,
                        )
                        .await;
                        if !clean {
                            leased.child.kill().await;
                        }
                        Ok(TurnEnd::Interrupted(SubagentStatus::TimedOut))
                    }
                }
            }
            None => {
                drive_turn(
                    &mut leased.child,
                    forwarder,
                    &send_id,
                    tracker,
                    self.cancel_grace,
                )
                .await
            }
        }
    }

    /// Терминальный исход без запуска turn-а (отмена до аренды процесса).
    async fn finish_interrupted(
        &self,
        spec: &SubagentRoleSpec,
        ctx: &RuntimeContext,
        child_thread_id: ThreadId,
        status: SubagentStatus,
    ) -> Result<SubagentResult> {
        ctx.emit(Event::SubagentFinished {
            role: spec.name.clone(),
            status: subagent_status_label(status),
            iterations: 0,
            child_thread_id,
        })
        .await?;
        Ok(SubagentResult::new(String::new(), status, 0)
            .with_child_thread_id(child_thread_id)
            .with_metadata(json!({ "resumable": false })))
    }

    /// Тело запуска: аренда процесса (в пределах permit-а роли), turn,
    /// возврат процесса в пул, resumable-учёт и `SubagentFinished`. Живёт
    /// detached tokio-таской; отменяется через child-токен `child_ctx`.
    async fn execute(
        self: Arc<Self>,
        spec: SubagentRoleSpec,
        request: SubagentRequest,
        ctx: RuntimeContext,
        child_ctx: RuntimeContext,
        child_thread_id: ThreadId,
        resume_process_id: Option<u64>,
    ) -> Result<SubagentResult> {
        let role = self
            .roles
            .get(&spec.name)
            .ok_or_else(|| anyhow!("unknown subagent role: {}", spec.name))?;
        let is_resume = resume_process_id.is_some();

        let permit = tokio::select! {
            permit = role.permits.clone().acquire_owned() => {
                permit.map_err(|_| anyhow!("subagent role process pool is closed"))?
            }
            _ = child_ctx.cancellation.cancelled() => {
                return self
                    .finish_interrupted(&spec, &ctx, child_thread_id, SubagentStatus::Cancelled)
                    .await;
            }
        };

        let mut tracker = TurnTracker::default();
        let forwarder = ChildEventForwarder {
            ctx: &child_ctx,
            child_thread_id,
            role: spec.name.clone(),
        };

        let mut leased = match self.lease_process(role, resume_process_id, &request.task.cwd) {
            Ok(leased) => leased,
            Err(error) => {
                drop(permit);
                let _ = ctx
                    .emit(Event::SubagentFinished {
                        role: spec.name.clone(),
                        status: "errored".into(),
                        iterations: 0,
                        child_thread_id,
                    })
                    .await;
                return Err(error);
            }
        };

        let body_result = self
            .run_leased_turn(
                &spec,
                &request,
                &forwarder,
                &mut tracker,
                &mut leased,
                is_resume,
            )
            .await;

        match body_result {
            Ok(end) => {
                leased.used = true;
                let child_alive = leased.child.is_alive();
                let process_id = leased.id;

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
                            process_id,
                        },
                    )?;
                }
                // Процесс возвращается в пул до освобождения permit-а,
                // чтобы следующий ожидающий нашёл его в idle, а не спавнил
                // лишний процесс сверх max_processes.
                self.release_process(role, leased, child_alive);
                drop(permit);

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
                // невалидным — убиваем и хороним вместе с его task_id-ами.
                leased.child.kill().await;
                self.release_process(role, leased, false);
                drop(permit);
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

#[async_trait]
impl SubagentRunner for ProcessSubagentRunner {
    fn roles(&self) -> Vec<SubagentRoleSpec> {
        self.inner.specs.clone()
    }

    /// `run` = `spawn` + `wait`: turn ребёнка живёт detached-таской, поэтому
    /// обрыв родительского future (отмена turn'а на границе block_on) не
    /// бросает процесс ребёнка без cancel-протокола — таска доводит
    /// `Cancel` + grace + kill и возвращает процесс в пул.
    async fn run(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentResult> {
        let handle = self.spawn(request, ctx).await?;
        self.wait(&handle).await
    }

    async fn spawn(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentHandle> {
        let prepared = self.inner.prepare(&request)?;
        let child_ctx = child_context(&ctx, prepared.child_thread_id, &prepared.spec.name);
        let spawn_id = new_call_id();
        self.inner.lock_pending()?.reserve(
            &spawn_id,
            child_ctx.cancellation.clone(),
            self.inner.max_parallel,
        )?;

        if let Err(error) = ctx
            .emit(Event::SubagentStarted {
                role: prepared.spec.name.clone(),
                description: request.description.clone(),
                child_thread_id: prepared.child_thread_id,
            })
            .await
        {
            self.inner.lock_pending()?.release(&spawn_id);
            return Err(error);
        }

        let handle = SubagentHandle::new(
            spawn_id.clone(),
            prepared.spec.name.clone(),
            prepared.child_thread_id,
        );
        let join = tokio::spawn(self.inner.clone().execute(
            prepared.spec,
            request,
            ctx.clone(),
            child_ctx,
            prepared.child_thread_id,
            prepared.resume_process_id,
        ));
        self.inner.lock_pending()?.attach(&spawn_id, join);
        Ok(handle)
    }

    async fn wait(&self, handle: &SubagentHandle) -> Result<SubagentResult> {
        let join = self.inner.lock_pending()?.take(&handle.spawn_id)?;
        join.await
            .map_err(|join_error| anyhow!("subagent child task failed: {join_error}"))?
    }

    async fn cancel(&self, handle: &SubagentHandle) -> Result<()> {
        self.inner.lock_pending()?.cancel(&handle.spawn_id)
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
/// `child_thread_id` и форвардинг интерактивных запросов. `ctx` — child
/// context запуска: те же transports/emitter, что у родителя, но
/// child-токен отмены (per-child cancel не задевает соседей).
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
    /// reply для ребёнка. `None` — запуск отменён во время ожидания решения.
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

    /// Форвардит typed user-input запрос ребёнка. `None` — запуск отменён.
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
/// форвардя события и интерактивные запросы. Отмена запуска (child-токен)
/// запускает cancel-протокол (`Cancel` → grace-дожидание Response → kill).
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
    /// Запуск отменён во время ожидания approval/user-input.
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
