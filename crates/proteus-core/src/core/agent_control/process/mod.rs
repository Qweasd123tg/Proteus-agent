//! Root-owned process agent control: ребёнок — отдельный
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
//! глобально bounded `idle` (LRU `max_idle_processes`) и получают
//! `ClearHistory` перед свежей задачей. Resume atomically резервирует idle
//! child и проверяет session/role/cwd. `run` = `spawn` + `wait`.

mod child;
mod config;
mod messaging;
mod outcome;
mod pool;
#[cfg(test)]
mod tests;
mod turn;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, MutexGuard},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use proteus_contracts::app_protocol::StdioRequest;
use serde_json::{Value, json};
use tokio::{sync::Semaphore, time::timeout};

use crate::{
    contracts::{
        AgentControl, AgentControlHandle, AgentControlMessage, AgentControlRequest,
        AgentControlResult, AgentLifecycleStatus, AgentProfile, RuntimeContext,
    },
    core::AgentControlConfig,
    domain::{Event, ThreadId, new_call_id, new_thread_id},
};

use super::{
    child_context, mailbox::ChildMailbox, pending::PendingChildren, requested_agent_target,
};
use config::{ProcessRoleConfig, build_agent_profiles};
use outcome::{status_label, truncate_summary};
use pool::{PooledChild, ProcessPool, ReleaseOutcome, ResumeReservation};
#[cfg(test)]
use turn::should_forward_child_event;
use turn::{
    ChildEventForwarder, TurnEnd, TurnTracker, cancel_child_turn, clear_child_history, drive_turn,
};

pub(super) struct ProcessAgentControl {
    inner: Arc<RunnerInner>,
}

struct RunnerInner {
    profiles: Vec<AgentProfile>,
    roles: HashMap<String, RoleState>,
    binary: PathBuf,
    max_depth: u64,
    max_parallel: usize,
    cancel_grace: Duration,
    pool: StdMutex<ProcessPool>,
    pending: StdMutex<PendingChildren>,
}

struct RoleState {
    config: ProcessRoleConfig,
    /// Конкурентное владение процессами роли: до `max_processes` детей
    /// одновременно; остальные ждут свободного permit-а.
    permits: Arc<Semaphore>,
}

/// Результат подготовки запуска (до `SubagentStarted`): спека роли, thread
/// ребёнка и resume-цель, если задан валидный `task_id`.
struct PreparedProcess {
    profile: AgentProfile,
    child_thread_id: ThreadId,
    resume: Option<ResumeReservation>,
}

impl ProcessAgentControl {
    pub(super) fn from_config(parsed: AgentControlConfig) -> Result<Self> {
        let profiles = build_agent_profiles(&parsed.roles)?;
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
                    },
                )
            })
            .collect();

        Ok(Self {
            inner: Arc::new(RunnerInner {
                profiles,
                roles,
                binary,
                max_depth: parsed.max_depth,
                max_parallel: parsed.max_parallel,
                cancel_grace: Duration::from_millis(parsed.cancel_grace_ms),
                pool: StdMutex::new(ProcessPool::new(parsed.max_idle_processes)),
                pending: StdMutex::new(PendingChildren::default()),
            }),
        })
    }
}

impl RunnerInner {
    fn lock_pending(&self) -> Result<MutexGuard<'_, PendingChildren>> {
        self.pending
            .lock()
            .map_err(|_| anyhow!("agent-control pending registry lock poisoned"))
    }

    fn lock_pool(&self) -> Result<MutexGuard<'_, ProcessPool>> {
        self.pool
            .lock()
            .map_err(|_| anyhow!("agent-control process pool lock poisoned"))
    }

    /// Резолвит роль, глубину и атомарно резервирует resume-цель до запуска:
    /// все ошибки подготовки возвращаются из `spawn` напрямую, ещё до
    /// `SubagentStarted`. Reserved child исключён из fresh reuse и eviction,
    /// пока detached execution ждёт role permit.
    fn prepare(
        &self,
        request: &AgentControlRequest,
        ctx: &RuntimeContext,
    ) -> Result<PreparedProcess> {
        let profile = self
            .profiles
            .iter()
            .find(|spec| spec.name == request.role)
            .cloned()
            .ok_or_else(|| anyhow!("unknown agent profile: {}", request.role))?;
        if !self.roles.contains_key(&request.role) {
            bail!("unknown agent profile: {}", request.role);
        }

        let depth = request
            .metadata
            .get("agent_control_depth")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if depth >= self.max_depth {
            bail!(
                "agent-control depth limit reached (depth {depth}, max_depth {})",
                self.max_depth
            );
        }

        let (child_thread_id, resume) =
            match request.metadata.get("task_id").and_then(Value::as_str) {
                Some(task_id) => {
                    let child_thread_id = task_id.parse::<ThreadId>().with_context(|| {
                        format!("invalid task_id for resumable subagent: {task_id}")
                    })?;
                    let reservation = self.lock_pool()?.reserve_resume(
                        task_id,
                        ctx.session_id,
                        &request.role,
                        &request.task.cwd,
                    )?;
                    (child_thread_id, Some(reservation))
                }
                None => (new_thread_id(), None),
            };

        Ok(PreparedProcess {
            profile,
            child_thread_id,
            resume,
        })
    }

    /// Берёт процесс роли в аренду. Resume использует заранее atomically
    /// reserved child; fresh — любой свободный same-role/same-cwd процесс либо
    /// новый (permit уже ограничил количество одновременных аренд).
    fn lease_process(
        &self,
        role_name: &str,
        role: &RoleState,
        resume: Option<&ResumeReservation>,
        cwd: &std::path::Path,
    ) -> Result<PooledChild> {
        let mut pool = self.lock_pool()?;
        match resume {
            Some(reservation) => pool.lease_reserved(reservation),
            None => pool.lease_fresh(&self.binary, role_name, &role.config, cwd),
        }
    }

    async fn rollback_resume(&self, reservation: &ResumeReservation) -> Result<bool> {
        let outcome = self.lock_pool()?.cancel_reservation(reservation)?;
        terminate_evicted(outcome.evicted).await;
        Ok(outcome.retained)
    }

    async fn release_process(
        &self,
        leased: PooledChild,
        alive: bool,
        session_id: crate::domain::SessionId,
        task_id: String,
    ) -> Result<bool> {
        let ReleaseOutcome { retained, evicted } = self
            .lock_pool()?
            .release(leased, alive, session_id, task_id);
        terminate_evicted(evicted).await;
        Ok(retained)
    }

    fn discard_process(&self, leased: PooledChild) -> Result<()> {
        self.lock_pool()?.discard(leased);
        Ok(())
    }

    /// Turn на арендованном процессе: опциональный ClearHistory, `Send`,
    /// затем drive до терминального Response с уважением role-таймаута.
    async fn run_leased_turn(
        &self,
        profile: &AgentProfile,
        request: &AgentControlRequest,
        forwarder: &ChildEventForwarder<'_>,
        tracker: &mut TurnTracker,
        leased: &mut PooledChild,
        mailbox: &ChildMailbox,
        is_resume: bool,
    ) -> Result<TurnEnd> {
        leased.child.drain_stale_outputs();
        if !is_resume && leased.used {
            clear_child_history(&mut leased.child).await?;
            // История процесса очищена — прежние task_id-ы этого процесса
            // мертвы, resume по ним продолжил бы пустую session.
            self.lock_pool()?.invalidate_history(leased.id);
        }

        let text = request.prompt.clone();
        let send_id = new_call_id();
        leased
            .child
            .send(&StdioRequest::Send {
                id: Some(send_id.clone()),
                text,
            })
            .await?;
        let active_send_id = StdMutex::new(send_id.clone());

        match self
            .roles
            .get(&profile.name)
            .and_then(|role| role.config.timeout_ms)
        {
            Some(timeout_ms) => {
                match timeout(
                    Duration::from_millis(timeout_ms),
                    drive_turn(
                        &mut leased.child,
                        forwarder,
                        &send_id,
                        tracker,
                        self.cancel_grace,
                        mailbox,
                        &active_send_id,
                    ),
                )
                .await
                {
                    Ok(end) => end,
                    Err(_elapsed) => {
                        let active_send_id = active_send_id
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .clone();
                        let clean = cancel_child_turn(
                            &mut leased.child,
                            forwarder,
                            &active_send_id,
                            tracker,
                            self.cancel_grace,
                        )
                        .await;
                        if !clean {
                            leased.child.kill().await;
                        }
                        Ok(TurnEnd::Interrupted(AgentLifecycleStatus::TimedOut))
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
                    mailbox,
                    &active_send_id,
                )
                .await
            }
        }
    }

    /// Терминальный исход без запуска turn-а (отмена до аренды процесса).
    async fn finish_interrupted(
        &self,
        profile: &AgentProfile,
        ctx: &RuntimeContext,
        child_thread_id: ThreadId,
        status: AgentLifecycleStatus,
        resumable: bool,
    ) -> Result<AgentControlResult> {
        ctx.emit(Event::SubagentFinished {
            role: profile.name.clone(),
            status: status_label(status),
            iterations: 0,
            child_thread_id,
        })
        .await?;
        Ok(AgentControlResult::new(String::new(), status)
            .with_child_thread_id(child_thread_id)
            .with_metadata(json!({ "resumable": resumable })))
    }

    /// Тело запуска: аренда процесса (в пределах permit-а роли), turn,
    /// возврат процесса в пул, resumable-учёт и `SubagentFinished`. Живёт
    /// detached tokio-таской; отменяется через child-токен `child_ctx`.
    async fn execute(
        self: Arc<Self>,
        profile: AgentProfile,
        request: AgentControlRequest,
        ctx: RuntimeContext,
        child_ctx: RuntimeContext,
        child_thread_id: ThreadId,
        resume: Option<ResumeReservation>,
        mailbox: Arc<ChildMailbox>,
    ) -> Result<AgentControlResult> {
        let role = self
            .roles
            .get(&profile.name)
            .ok_or_else(|| anyhow!("unknown agent profile: {}", profile.name))?;
        let is_resume = resume.is_some();

        let permit = tokio::select! {
            permit = role.permits.clone().acquire_owned() => {
                permit.map_err(|_| anyhow!("agent profile process pool is closed"))?
            }
            _ = child_ctx.cancellation.cancelled() => {
                let _ = mailbox.close_and_discard();
                let resumable = match resume.as_ref() {
                    Some(reservation) => self.rollback_resume(reservation).await?,
                    None => false,
                };
                return self
                    .finish_interrupted(
                        &profile,
                        &ctx,
                        child_thread_id,
                        AgentLifecycleStatus::Cancelled,
                        resumable,
                    )
                    .await;
            }
        };

        let mut tracker = TurnTracker::default();
        let forwarder = ChildEventForwarder {
            ctx: &child_ctx,
            child_thread_id,
            role: profile.name.clone(),
        };

        let mut leased =
            match self.lease_process(&profile.name, role, resume.as_ref(), &request.task.cwd) {
                Ok(leased) => leased,
                Err(error) => {
                    let _ = mailbox.close_and_discard();
                    drop(permit);
                    let _ = ctx
                        .emit(Event::SubagentFinished {
                            role: profile.name.clone(),
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
                &profile,
                &request,
                &forwarder,
                &mut tracker,
                &mut leased,
                &mailbox,
                is_resume,
            )
            .await;

        if !matches!(&body_result, Ok(TurnEnd::Completed(_))) {
            let _ = mailbox.close_and_discard();
        }

        match body_result {
            Ok(end) => {
                leased.used = true;
                let child_alive = leased.child.is_alive();

                let (status, summary) = match end {
                    TurnEnd::Completed(text) => (AgentLifecycleStatus::Completed, text),
                    TurnEnd::Interrupted(status) => (status, tracker.partial_text()),
                };
                let mut summary = summary;
                if let Some(max_bytes) = role.config.max_summary_bytes {
                    summary = truncate_summary(summary, max_bytes);
                }

                // Процесс возвращается в пул до освобождения permit-а,
                // чтобы следующий ожидающий нашёл его в idle, а не спавнил
                // лишний процесс сверх max_processes.
                let resumable = self
                    .release_process(
                        leased,
                        child_alive,
                        ctx.session_id,
                        child_thread_id.to_string(),
                    )
                    .await?;
                drop(permit);

                ctx.emit(Event::SubagentFinished {
                    role: profile.name.clone(),
                    status: status_label(status),
                    iterations: tracker.iterations,
                    child_thread_id,
                })
                .await?;

                let result = AgentControlResult::new(summary, status)
                    .with_child_thread_id(child_thread_id)
                    .with_metadata(json!({
                        "resumable": resumable,
                        "agent_messages_delivered": tracker.delivered_messages,
                    }));
                Ok(result)
            }
            Err(error) => {
                // Fatal-путь (EOF, ошибка child turn-а): процесс считается
                // невалидным — убиваем и хороним вместе с его task_id-ами.
                leased.child.kill().await;
                self.discard_process(leased)?;
                drop(permit);
                let _ = ctx
                    .emit(Event::SubagentFinished {
                        role: profile.name.clone(),
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
impl AgentControl for ProcessAgentControl {
    fn profiles(&self) -> Vec<AgentProfile> {
        self.inner.profiles.clone()
    }

    /// `run` = `spawn` + `wait`: turn ребёнка живёт detached-таской, поэтому
    /// обрыв родительского future (отмена turn'а на границе block_on) не
    /// бросает процесс ребёнка без cancel-протокола — таска доводит
    /// `Cancel` + grace + kill и возвращает процесс в пул.
    async fn run(
        &self,
        request: AgentControlRequest,
        ctx: RuntimeContext,
    ) -> Result<AgentControlResult> {
        let handle = self.spawn(request, ctx).await?;
        self.wait(&handle).await
    }

    async fn spawn(
        &self,
        request: AgentControlRequest,
        ctx: RuntimeContext,
    ) -> Result<AgentControlHandle> {
        let prepared = self.inner.prepare(&request, &ctx)?;
        let child_ctx = child_context(&ctx, prepared.child_thread_id, &prepared.profile.name);
        let spawn_id = new_call_id();
        let mailbox = Arc::new(ChildMailbox::default());
        let agent_target = requested_agent_target(&request)?;
        let reserve_result = self.inner.lock_pending()?.reserve(
            &spawn_id,
            child_ctx.cancellation.clone(),
            mailbox.clone(),
            agent_target,
            self.inner.max_parallel,
            !request
                .metadata
                .get("control_plane_owned")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        if let Err(error) = reserve_result {
            if let Some(resume) = prepared.resume.as_ref() {
                self.inner.rollback_resume(resume).await?;
            }
            return Err(error);
        }

        if let Err(error) = ctx
            .emit(Event::SubagentStarted {
                role: prepared.profile.name.clone(),
                description: request.description.clone(),
                child_thread_id: prepared.child_thread_id,
            })
            .await
        {
            self.inner.lock_pending()?.release(&spawn_id);
            if let Some(resume) = prepared.resume.as_ref() {
                self.inner.rollback_resume(resume).await?;
            }
            return Err(error);
        }

        let handle = AgentControlHandle::new(
            spawn_id.clone(),
            prepared.profile.name.clone(),
            prepared.child_thread_id,
        );
        let join = tokio::spawn(self.inner.clone().execute(
            prepared.profile,
            request,
            ctx.clone(),
            child_ctx,
            prepared.child_thread_id,
            prepared.resume,
            mailbox,
        ));
        self.inner.lock_pending()?.attach(&spawn_id, join);
        Ok(handle)
    }

    async fn wait(&self, handle: &AgentControlHandle) -> Result<AgentControlResult> {
        let outcome = self.inner.lock_pending()?.outcome(&handle.spawn_id)?;
        let result = outcome.wait().await;
        self.inner.lock_pending()?.consume(&handle.spawn_id)?;
        result
    }

    async fn cancel(&self, handle: &AgentControlHandle) -> Result<()> {
        let mailbox = self
            .inner
            .lock_pending()?
            .begin_cancel_and_close_mailbox(&handle.spawn_id)?;
        mailbox.wait_for_delivery_idle().await;
        Ok(())
    }

    async fn send(&self, handle: &AgentControlHandle, message: AgentControlMessage) -> Result<()> {
        self.inner.lock_pending()?.send(&handle.spawn_id, message)?;
        Ok(())
    }
}

async fn terminate_evicted(children: Vec<PooledChild>) {
    for mut child in children {
        child.child.kill().await;
    }
}
