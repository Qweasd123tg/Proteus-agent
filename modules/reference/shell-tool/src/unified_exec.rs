//! Persistent terminal tools: pipes by default, PTY with explicit `tty=true`.
//! Launch, session ownership and output collection are separate local modules.

use crate::sandbox::{SandboxKind, SandboxPolicy, resolve_workdir};
use anyhow::{Context, Result, anyhow};
use proteus_contracts::process_module::{ToolModuleHostMut, ToolModuleInvocationContext};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const DEFAULT_EXEC_YIELD_MS: u64 = 10_000;
const DEFAULT_WRITE_YIELD_MS: u64 = 250;
const MIN_YIELD_MS: u64 = 250;
/// Пустой `chars` — это poll; заставляем модель ждать заметное время вместо
/// busy-loop из коротких пустых вызовов (Codex-семантика).
const MIN_EMPTY_WRITE_YIELD_MS: u64 = 5_000;
const MAX_YIELD_MS: u64 = 30_000;
/// Спековый timeout: max yield + запас на spawn/drain.
const EXEC_SPEC_TIMEOUT_MS: u64 = 60_000;
const WRITE_SPEC_TIMEOUT_MS: u64 = 330_000;
const MAX_EMPTY_WRITE_YIELD_MS: u64 = 300_000;
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 10_000;
const APPROX_BYTES_PER_TOKEN: u64 = 4;
/// Непрочитанный вывод сессии между вызовами; сохраняются начало и конец.
const SESSION_BUFFER_LIMIT: usize = 1024 * 1024;
const MAX_SESSIONS: usize = 16;
const SESSION_MAX_IDLE: Duration = Duration::from_secs(30 * 60);
const SESSION_JANITOR_INTERVAL: Duration = Duration::from_secs(60);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// После выхода процесса даём reader'у дочитать хвост из PTY.
const EXIT_DRAIN_GRACE: Duration = Duration::from_millis(50);

mod output;
mod process;
mod session;
mod spec;

use output::*;
use process::spawn_session;
use session::*;
pub(crate) use spec::{ExecCommandTool, WriteStdinTool};

fn exec_command_impl(
    call_json: &str,
    context_json: &str,
    host: &mut ToolModuleHostMut<'_>,
) -> Result<String> {
    ensure_not_cancelled(host)?;
    let call: Value =
        serde_json::from_str(call_json).with_context(|| "failed to parse ToolCall JSON")?;
    let context: ToolModuleInvocationContext = serde_json::from_str(context_json)
        .with_context(|| "failed to parse ToolModuleInvocationContext")?;
    let call_id = call
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let args = call.get("args");
    let cmd = args
        .and_then(|args| args.get("cmd"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("exec_command requires string arg 'cmd'"))?;
    let escalated = args
        .and_then(|args| args.get("with_escalated_permissions"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sandbox_policy = if escalated {
        SandboxPolicy::not_required()
    } else {
        SandboxPolicy::detect(context.cwd.to_string_lossy().as_ref())
    };
    execute_command(
        call_id,
        args,
        cmd,
        &context,
        escalated,
        sandbox_policy,
        host,
    )
}

fn execute_command(
    call_id: String,
    args: Option<&Value>,
    cmd: &str,
    context: &ToolModuleInvocationContext,
    escalated: bool,
    sandbox_policy: SandboxPolicy,
    host: &mut ToolModuleHostMut<'_>,
) -> Result<String> {
    ensure_not_cancelled(host)?;
    let cwd = context.cwd.to_string_lossy();
    let resolved = resolve_workdir(
        cwd.as_ref(),
        args.and_then(|args| args.get("workdir")),
        escalated,
    )?;
    let yield_time_ms = resolve_yield_time_ms(args, DEFAULT_EXEC_YIELD_MS);
    let max_output_bytes = resolve_max_output_bytes(args);
    let tty = args
        .and_then(|args| args.get("tty"))
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| anyhow!("exec_command arg 'tty' must be a boolean"))
        })
        .transpose()?
        .unwrap_or(false);
    let sandbox = sandbox_policy.select(escalated, false)?;

    let started = Instant::now();
    let (session_id, session) = spawn_session(
        cmd,
        tty,
        &resolved.workspace,
        &resolved.workdir,
        sandbox,
        ExecSessionOwner::from_context(context, &resolved.workspace),
    )?;
    let collected = match wait_and_collect(&session, Duration::from_millis(yield_time_ms), host) {
        Ok(collected) => collected,
        Err(error) => {
            terminate_session(session_id, &session);
            return Err(error);
        }
    };
    let wall_time = started.elapsed();
    if collected.exited {
        lock(sessions()).remove(&session_id);
    }

    let metadata = json!({
        "yield_time_ms": yield_time_ms,
        "workdir": resolved.workdir,
        "sandbox": session.sandbox.as_ref().map(SandboxKind::label),
        "escalated": escalated,
        "tty": tty,
    });
    Ok(render_result(
        &call_id,
        session_id,
        collected,
        wall_time,
        max_output_bytes,
        metadata,
    ))
}

fn write_stdin_impl(
    call_json: &str,
    context_json: &str,
    host: &mut ToolModuleHostMut<'_>,
) -> Result<String> {
    ensure_not_cancelled(host)?;
    let call: Value =
        serde_json::from_str(call_json).with_context(|| "failed to parse ToolCall JSON")?;
    let context: ToolModuleInvocationContext = serde_json::from_str(context_json)
        .with_context(|| "failed to parse ToolModuleInvocationContext")?;
    let call_id = call
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let args = call.get("args");
    let session_id = args
        .and_then(|args| args.get("session_id"))
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("write_stdin requires integer arg 'session_id'"))?;
    let chars = args
        .and_then(|args| args.get("chars"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let yield_time_ms = resolve_write_yield_time_ms(args, chars);
    let max_output_bytes = resolve_max_output_bytes(args);

    let session = {
        let sessions = lock(sessions());
        let session = sessions.get(&session_id).cloned().ok_or_else(|| {
            anyhow!("unknown exec session {session_id}; the process may have already exited")
        })?;
        if !session.owner.matches(&context) {
            anyhow::bail!(
                "exec session {session_id} is not owned by the current execution context/workspace"
            );
        }
        session.touch();
        session
    };

    if !chars.is_empty() {
        if !session.tty {
            if chars != "\u{3}" {
                anyhow::bail!(
                    "stdin is closed for this session; rerun exec_command with tty=true to keep stdin open"
                );
            }
            session
                .control
                .interrupt()
                .context("failed to interrupt session")?;
        } else {
            match session.control.write(chars.as_bytes()) {
                Ok(()) => {
                    // Pinned Codex gives the PTY 100ms to react before polling.
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) if !lock(&session.output).exited => {
                    return Err(anyhow!(error).context("failed to write to session stdin"));
                }
                Err(_) => {} // Report the known exit when a write races with EOF.
            }
        }
    }
    let started = Instant::now();
    let collected = match wait_and_collect(&session, Duration::from_millis(yield_time_ms), host) {
        Ok(collected) => collected,
        Err(error) => {
            terminate_session(session_id, &session);
            return Err(error);
        }
    };
    let wall_time = started.elapsed();
    if collected.exited {
        lock(sessions()).remove(&session_id);
    }

    let metadata = json!({
        "yield_time_ms": yield_time_ms,
        "stdin_bytes": chars.len(),
        "tty": session.tty,
        "sandbox": session.sandbox.as_ref().map(SandboxKind::label),
    });
    Ok(render_result(
        &call_id,
        session_id,
        collected,
        wall_time,
        max_output_bytes,
        metadata,
    ))
}

fn resolve_yield_time_ms(args: Option<&Value>, default_ms: u64) -> u64 {
    args.and_then(|args| args.get("yield_time_ms"))
        .and_then(Value::as_u64)
        .map_or(default_ms, |requested| {
            requested.clamp(MIN_YIELD_MS, MAX_YIELD_MS)
        })
}

fn resolve_write_yield_time_ms(args: Option<&Value>, chars: &str) -> u64 {
    let requested = args
        .and_then(|args| args.get("yield_time_ms"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WRITE_YIELD_MS);
    if chars.is_empty() {
        requested.clamp(MIN_EMPTY_WRITE_YIELD_MS, MAX_EMPTY_WRITE_YIELD_MS)
    } else {
        requested.clamp(MIN_YIELD_MS, MAX_YIELD_MS)
    }
}

fn resolve_max_output_bytes(args: Option<&Value>) -> usize {
    let tokens = args
        .and_then(|args| args.get("max_output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
    usize::try_from(tokens.saturating_mul(APPROX_BYTES_PER_TOKEN)).unwrap_or(usize::MAX)
}

fn ensure_not_cancelled(host: &mut ToolModuleHostMut<'_>) -> Result<()> {
    if invocation_is_cancelled(host)? {
        anyhow::bail!("tool invocation canceled");
    }
    Ok(())
}

fn invocation_is_cancelled(host: &mut ToolModuleHostMut<'_>) -> Result<bool> {
    match host.is_cancelled() {
        Ok(cancelled) => Ok(cancelled),
        Err(error) => Err(anyhow!(
            "failed to query module cancellation: {}",
            error.message
        )),
    }
}

#[cfg(test)]
mod tests;
