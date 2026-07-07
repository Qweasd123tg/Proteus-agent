//! Builtin slot `subagent`: последовательный дочерний агентский цикл.
//!
//! `SequentialSubagentRunner` владеет циклом ребёнка целиком
//! (модель → tools → модель), не вызывая slot `workflow`. Ребёнок изолирован:
//! свой `ThreadId`, свой `CancellationToken` (child-токен родительского),
//! своя история (только `role.prompt` + `request.prompt`), свой отбор tools
//! по фазе роли. Tool calls ребёнка идут через тот же `ToolOrchestrator`
//! (policy/approval-контур), что и родительские.

mod child_loop;
mod process;
mod resumable;
mod roles;
#[cfg(test)]
mod tests;

pub use process::ProcessSubagentRunner;

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::time::timeout;

use crate::{
    contracts::{
        RuntimeContext, SubagentRequest, SubagentResult, SubagentRoleSpec, SubagentRunner,
        SubagentStatus,
    },
    core::ToolOrchestrator,
    domain::{Event, SessionId, ThreadId, new_thread_id},
    model_standard::{CanonicalMessage, MessageRole},
};

use child_loop::{
    ChildLoopState, run_child_loop, select_child_tools, subagent_status_label,
    truncate_at_char_boundary,
};
use resumable::{ResumableSnapshot, ResumableStore};
use roles::{SequentialSubagentConfig, build_role_specs};

/// Имя tool'а делегирования, который workflow генерирует из ролей.
/// Убирается из тулсета ребёнка, чтобы запретить рекурсию на уровне тулсета.
const TASK_TOOL_NAME: &str = "task";

#[derive(Debug)]
pub struct SequentialSubagentRunner {
    roles: Vec<SubagentRoleSpec>,
    max_depth: u64,
    max_resumable: usize,
    resumable: Mutex<ResumableStore>,
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

        let (roles, max_depth, max_resumable) = build_role_specs(parsed, cwd)?;
        Ok(Self {
            roles,
            max_depth,
            max_resumable,
            resumable: Mutex::new(ResumableStore::default()),
        })
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
}

#[async_trait]
impl SubagentRunner for SequentialSubagentRunner {
    fn roles(&self) -> Vec<SubagentRoleSpec> {
        self.roles.clone()
    }

    async fn run(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentResult> {
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
        let child_ctx = child_context(&ctx, child_thread_id, &role.name);

        // Started/Finished эмитятся под родительским thread_id; события
        // самого цикла (tool calls) — под child_thread_id через child_ctx.
        ctx.emit(Event::SubagentStarted {
            role: role.name.clone(),
            description: request.description.clone(),
            child_thread_id,
        })
        .await?;

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

/// Контекст дочернего цикла поверх родительского: собственный `thread_id`,
/// метка роли для attribution (approvals, user inputs, клиентский UX),
/// пустые turn-scoped grants и child-токен отмены. Изоляция grants
/// структурная: ребёнок не наследует права родителя (например,
/// `escalated_exec` после approved-запроса) и не протаскивает свои granted
/// permissions обратно в родительский ход. Child-токен: cancel родителя
/// каскадится ребёнку, но ребёнка можно отменить отдельно, не трогая
/// родительский turn (groundwork для parallel subagents).
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
