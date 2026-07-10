//! Builtin slot `subagent`: последовательный дочерний агентский цикл.
//!
//! `SequentialSubagentRunner` владеет циклом ребёнка целиком
//! (модель → tools → модель), не вызывая slot `workflow`. Ребёнок изолирован:
//! свой `ThreadId`, свой `CancellationToken` (child-токен родительского),
//! своя история (только `role.prompt` + `request.prompt`), свой отбор tools
//! по фазе роли. Tool calls ребёнка идут через тот же `ToolOrchestrator`
//! (policy/approval-контур), что и родительские.
//!
//! Исполнение — через `spawn`/`wait`/`cancel`: ребёнок живёт detached
//! tokio-таской в реестре `PendingChildren`, поэтому несколько детей могут
//! работать конкурентно, а обрыв родительского future не роняет цикл
//! ребёнка на полпути. `run` = `spawn` + `wait`.

mod child_loop;
mod pending;
mod process;
mod resumable;
mod roles;
#[cfg(test)]
mod tests;

pub use process::ProcessSubagentRunner;

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::time::timeout;

use crate::{
    contracts::{
        RuntimeContext, SubagentHandle, SubagentRequest, SubagentResult, SubagentRoleSpec,
        SubagentRunner, SubagentStatus,
    },
    core::ToolOrchestrator,
    domain::{Event, SessionId, ThreadId, new_call_id, new_thread_id},
    model_standard::{CanonicalMessage, MessageRole},
};

use child_loop::{
    ChildLoopState, run_child_loop, select_child_tools, subagent_status_label,
    truncate_at_char_boundary,
};
use pending::PendingChildren;
use resumable::{ResumableSnapshot, ResumableStore};
use roles::{SequentialSubagentConfig, build_role_specs};

/// Facade tools are removed from child toolsets to keep the first
/// collaboration slice root-owned and non-recursive.
const SUBAGENT_FACADE_TOOLS: &[&str] = &[
    "task",
    "spawn_agent",
    "list_agents",
    "wait_agent",
    "interrupt_agent",
];

pub struct SequentialSubagentRunner {
    inner: Arc<RunnerInner>,
}

impl std::fmt::Debug for SequentialSubagentRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SequentialSubagentRunner")
            .field("roles", &self.inner.roles.len())
            .field("max_depth", &self.inner.max_depth)
            .field("max_parallel", &self.inner.max_parallel)
            .finish_non_exhaustive()
    }
}

struct RunnerInner {
    roles: Vec<SubagentRoleSpec>,
    max_depth: u64,
    max_resumable: usize,
    max_parallel: usize,
    resumable: Mutex<ResumableStore>,
    pending: Mutex<PendingChildren>,
}

/// Результат подготовки запуска: роль, thread ребёнка и стартовая история
/// (fresh: prompt роли + задача; resume: снапшот + новая задача).
struct PreparedChild {
    role: SubagentRoleSpec,
    child_thread_id: ThreadId,
    history: Vec<CanonicalMessage>,
}

impl SequentialSubagentRunner {
    /// Строит runner из значения `module_config.subagent.sequential`.
    /// `Null` (конфига нет) — валидно: ролей нет, делегирование выключено.
    pub fn from_config(config: Value) -> Result<Self> {
        let cwd = std::env::current_dir().context("failed to resolve process cwd")?;
        Self::from_config_with_cwd(config, &cwd)
    }

    pub fn from_config_with_cwd(config: Value, cwd: &Path) -> Result<Self> {
        let parsed: SequentialSubagentConfig = if config.is_null() {
            SequentialSubagentConfig::default()
        } else {
            serde_json::from_value(config)
                .context("failed to parse module_config.subagent.sequential")?
        };

        let (roles, max_depth, max_resumable, max_parallel) = build_role_specs(parsed, cwd)?;
        Ok(Self {
            inner: Arc::new(RunnerInner {
                roles,
                max_depth,
                max_resumable,
                max_parallel,
                resumable: Mutex::new(ResumableStore::default()),
                pending: Mutex::new(PendingChildren::default()),
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

    fn resumable_snapshot(&self, task_id: &str) -> Result<Option<ResumableSnapshot>> {
        Ok(self
            .resumable
            .lock()
            .map_err(|_| anyhow!("subagent resumable store lock poisoned"))?
            .get(task_id))
    }

    fn save_resumable_snapshot(
        &self,
        child_thread_id: ThreadId,
        session_id: SessionId,
        role_name: String,
        history: Vec<CanonicalMessage>,
    ) -> Result<bool> {
        Ok(self
            .resumable
            .lock()
            .map_err(|_| anyhow!("subagent resumable store lock poisoned"))?
            .save(
                child_thread_id.to_string(),
                session_id,
                role_name,
                history,
                self.max_resumable,
            ))
    }

    /// Резолвит роль, глубину и resume до запуска: все ошибки подготовки
    /// возвращаются из `spawn` напрямую, ещё до `SubagentStarted`.
    fn prepare(&self, request: &SubagentRequest, ctx: &RuntimeContext) -> Result<PreparedChild> {
        let role = self
            .roles
            .iter()
            .find(|role| role.name == request.role)
            .cloned()
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

        let resume_task_id = request.metadata.get("task_id").and_then(Value::as_str);
        let (child_thread_id, history) = if let Some(task_id) = resume_task_id {
            let snapshot = self
                .resumable_snapshot(task_id)?
                .ok_or_else(|| anyhow!("unknown task_id (expired or from another session)"))?;
            if snapshot.session_id != ctx.session_id {
                bail!("unknown task_id (expired or from another session)");
            }
            if snapshot.role_name != request.role {
                bail!(
                    "task_id belongs to subagent role {}, but request role is {}",
                    snapshot.role_name,
                    request.role
                );
            }
            let child_thread_id = task_id
                .parse::<ThreadId>()
                .with_context(|| format!("invalid task_id for resumable subagent: {task_id}"))?;
            let mut history = snapshot.history;
            history.push(CanonicalMessage::text(
                MessageRole::User,
                request.prompt.clone(),
            ));
            (child_thread_id, history)
        } else {
            (
                new_thread_id(),
                vec![
                    CanonicalMessage::text(MessageRole::System, role.prompt.clone()),
                    CanonicalMessage::text(MessageRole::User, request.prompt.clone()),
                ],
            )
        };

        Ok(PreparedChild {
            role,
            child_thread_id,
            history,
        })
    }

    /// Тело дочернего цикла: от подготовленной истории до терминального
    /// статуса, resumable snapshot и `SubagentFinished`. Живёт detached
    /// tokio-таской (`spawn`), поэтому события эмитятся через собственные
    /// клоны контекстов, а не через ссылки на caller.
    async fn execute(
        self: Arc<Self>,
        role: SubagentRoleSpec,
        request: SubagentRequest,
        ctx: RuntimeContext,
        child_ctx: RuntimeContext,
        child_thread_id: ThreadId,
        history: Vec<CanonicalMessage>,
    ) -> Result<SubagentResult> {
        // Ребёнок не наследует родительские ctx.instructions: system-слой —
        // только prompt роли.
        let mut state = ChildLoopState {
            history,
            iterations: 0,
            last_text: None,
            usage: None,
        };

        let body_result: Result<(SubagentResult, String)> = async {
            let orchestrator = ToolOrchestrator::default();
            let tools = select_child_tools(&child_ctx, &orchestrator, &request, &role).await?;

            let status = match role.limits.timeout_ms {
                Some(timeout_ms) => {
                    match timeout(
                        Duration::from_millis(timeout_ms),
                        run_child_loop(
                            &role,
                            &request,
                            &child_ctx,
                            &orchestrator,
                            &tools,
                            &mut state,
                        ),
                    )
                    .await
                    {
                        Ok(status) => status?,
                        Err(_elapsed) => SubagentStatus::TimedOut,
                    }
                }
                None => {
                    run_child_loop(
                        &role,
                        &request,
                        &child_ctx,
                        &orchestrator,
                        &tools,
                        &mut state,
                    )
                    .await?
                }
            };

            let status_label = subagent_status_label(status);
            let mut summary = state.last_text.clone().unwrap_or_default();
            if let Some(max_bytes) = role.limits.max_summary_bytes {
                summary = truncate_at_char_boundary(summary, max_bytes);
            }

            let mut result = SubagentResult::new(summary, status, state.iterations)
                .with_child_thread_id(child_thread_id)
                .with_metadata(json!({ "resumable": false }));
            if let Some(usage) = state.usage.clone() {
                result = result.with_usage(usage);
            }
            Ok((result, status_label))
        }
        .await;

        match body_result {
            Ok((mut result, status)) => {
                // Snapshot сохраняется для любого терминального статуса,
                // включая Cancelled/TimedOut: прерванный ребёнок не должен
                // терять частичную работу — её можно продолжить по task_id
                // (dogfood-находка 2026-07-06, кластер 2 аудита).
                let resumable = self.save_resumable_snapshot(
                    child_thread_id,
                    ctx.session_id,
                    role.name.clone(),
                    state.history.clone(),
                )?;
                result.metadata = json!({ "resumable": resumable });
                ctx.emit(Event::SubagentFinished {
                    role: role.name.clone(),
                    status,
                    iterations: state.iterations,
                    child_thread_id,
                })
                .await?;
                Ok(result)
            }
            Err(error) => {
                let _ = ctx
                    .emit(Event::SubagentFinished {
                        role: role.name.clone(),
                        status: "errored".into(),
                        iterations: state.iterations,
                        child_thread_id,
                    })
                    .await;
                Err(error)
            }
        }
    }
}

#[async_trait]
impl SubagentRunner for SequentialSubagentRunner {
    fn roles(&self) -> Vec<SubagentRoleSpec> {
        self.inner.roles.clone()
    }

    fn supports_collaboration(&self) -> bool {
        true
    }

    /// `run` = `spawn` + `wait`: цикл ребёнка исполняется detached-таской,
    /// поэтому обрыв родительского future (например, отмена turn'а на
    /// границе block_on в workflow host) не роняет ребёнка на полпути —
    /// тот доводит работу до терминального статуса и сохраняет resumable
    /// snapshot.
    async fn run(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentResult> {
        let handle = self.spawn(request, ctx).await?;
        self.wait(&handle).await
    }

    async fn spawn(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentHandle> {
        let prepared = self.inner.prepare(&request, &ctx)?;
        let child_ctx = child_context(&ctx, prepared.child_thread_id, &prepared.role.name);
        let spawn_id = new_call_id();
        self.inner.lock_pending()?.reserve(
            &spawn_id,
            child_ctx.cancellation.clone(),
            self.inner.max_parallel,
            !request
                .metadata
                .get("control_plane_owned")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )?;

        // Started/Finished эмитятся под родительским thread_id; события
        // самого цикла (tool calls) — под child_thread_id через child_ctx.
        // Started уходит до tokio::spawn, чтобы события ребёнка не могли
        // обогнать его в event stream.
        if let Err(error) = ctx
            .emit(Event::SubagentStarted {
                role: prepared.role.name.clone(),
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
            prepared.role.name.clone(),
            prepared.child_thread_id,
        );
        let join = tokio::spawn(self.inner.clone().execute(
            prepared.role,
            request,
            ctx.clone(),
            child_ctx,
            prepared.child_thread_id,
            prepared.history,
        ));
        self.inner.lock_pending()?.attach(&spawn_id, join);
        Ok(handle)
    }

    async fn wait(&self, handle: &SubagentHandle) -> Result<SubagentResult> {
        let outcome = self.inner.lock_pending()?.outcome(&handle.spawn_id)?;
        let result = outcome.wait().await;
        self.inner.lock_pending()?.consume(&handle.spawn_id)?;
        result
    }

    async fn cancel(&self, handle: &SubagentHandle) -> Result<()> {
        self.inner.lock_pending()?.cancel(&handle.spawn_id)
    }
}

/// Контекст дочернего цикла поверх родительского: собственный `thread_id`,
/// метка роли для attribution (approvals, user inputs, клиентский UX),
/// пустые turn-scoped grants и child-токен отмены. Изоляция grants
/// структурная: ребёнок не наследует права родителя (например,
/// `escalated_exec` после approved-запроса) и не протаскивает свои granted
/// permissions обратно в родительский ход. Child-токен: cancel родителя
/// каскадится ребёнку, но ребёнка можно отменить отдельно (`cancel` по
/// handle), не трогая родительский turn и соседних детей.
fn child_context(
    ctx: &RuntimeContext,
    child_thread_id: ThreadId,
    role_name: &str,
) -> RuntimeContext {
    let mut child_ctx = ctx.clone();
    child_ctx.thread_id = child_thread_id;
    child_ctx.thread_label = Some(role_name.to_owned());
    child_ctx.turn_grants = Arc::default();
    child_ctx.cancellation = ctx.cancellation.child_token();
    child_ctx
}
