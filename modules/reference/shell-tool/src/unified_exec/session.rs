//! Session ownership, storage and lifetime.
use super::{
    CANCELLATION_POLL_INTERVAL, EXIT_DRAIN_GRACE, MAX_SESSIONS, SESSION_BUFFER_LIMIT,
    SESSION_JANITOR_INTERVAL, SESSION_MAX_IDLE, ensure_not_cancelled,
};
use crate::sandbox::SandboxKind;
use anyhow::{Result, anyhow};
use proteus_contracts::{
    domain::{ExecutionId, SessionId, ThreadId},
    process_module::{ToolModuleHostMut, ToolModuleInvocationContext},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError, atomic::AtomicI64},
    time::{Duration, Instant},
};

/// Состояние вывода сессии; reader/wait-потоки будят ожидающих через Condvar.
pub(super) struct SessionOutput {
    pub(super) buffer: Vec<u8>,
    pub(super) dropped_bytes: usize,
    /// All output readers reached EOF.
    pub(super) closed: bool,
    readers: usize,
    pub(super) exited: bool,
    pub(super) exit_code: Option<i32>,
    exited_at: Option<Instant>,
}

pub(super) struct ExecSession {
    /// Serializes input, output collection and terminal removal for this handle.
    pub(super) interaction: Mutex<()>,
    pub(super) output: Mutex<SessionOutput>,
    pub(super) output_cond: Condvar,
    pub(super) control: Box<dyn ProcessControl>,
    pub(super) tty: bool,
    pub(super) sandbox: Option<SandboxKind>,
    pub(super) owner: ExecSessionOwner,
    /// Для LRU-prune: обновляется при каждом обращении к сессии.
    pub(super) last_used: Mutex<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExecSessionOwner {
    principal: ExecSessionPrincipal,
    workspace: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecSessionPrincipal {
    Agent {
        session_id: SessionId,
        thread_id: ThreadId,
    },
    DetachedExecution(ExecutionId),
}

impl ExecSessionOwner {
    pub(super) fn from_context(
        context: &ToolModuleInvocationContext,
        canonical_workspace: &str,
    ) -> Self {
        Self {
            principal: ExecSessionPrincipal::from_context(context),
            workspace: PathBuf::from(canonical_workspace),
        }
    }

    pub(super) fn matches(&self, context: &ToolModuleInvocationContext) -> bool {
        let Ok(workspace) = context.cwd.canonicalize() else {
            return false;
        };
        self.principal == ExecSessionPrincipal::from_context(context) && self.workspace == workspace
    }
}

impl ExecSessionPrincipal {
    pub(super) fn from_context(context: &ToolModuleInvocationContext) -> Self {
        context.attribution.agent.map_or(
            Self::DetachedExecution(context.attribution.execution_id),
            |agent| Self::Agent {
                session_id: agent.session_id,
                thread_id: agent.thread_id,
            },
        )
    }
}

/// Session lifetime depends on this private backend contract, not PTY internals.
pub(super) trait ProcessControl: Send + Sync {
    fn write(&self, bytes: &[u8]) -> std::io::Result<()>;
    fn interrupt(&self) -> std::io::Result<()>;
    fn kill(&self);
}

impl ExecSession {
    pub(super) fn new(
        control: Box<dyn ProcessControl>,
        tty: bool,
        sandbox: Option<SandboxKind>,
        owner: ExecSessionOwner,
    ) -> Self {
        Self {
            interaction: Mutex::new(()),
            output: Mutex::new(SessionOutput {
                buffer: Vec::new(),
                dropped_bytes: 0,
                closed: false,
                readers: if tty { 1 } else { 2 },
                exited: false,
                exit_code: None,
                exited_at: None,
            }),
            output_cond: Condvar::new(),
            control,
            tty,
            sandbox,
            owner,
            last_used: Mutex::new(Instant::now()),
        }
    }

    pub(super) fn touch(&self) {
        *lock(&self.last_used) = Instant::now();
    }

    pub(super) fn lock_interaction(
        &self,
        host: &mut ToolModuleHostMut<'_>,
    ) -> Result<MutexGuard<'_, ()>> {
        loop {
            // Queued cancellation must settle without blocking behind a long
            // poll or terminating the session held by a different invocation.
            ensure_not_cancelled(host)?;
            if let Some(guard) = self.try_lock_interaction() {
                return Ok(guard);
            }
            std::thread::sleep(CANCELLATION_POLL_INTERVAL);
        }
    }

    fn try_lock_interaction(&self) -> Option<MutexGuard<'_, ()>> {
        match self.interaction.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(error)) => Some(error.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    pub(super) fn push_output(&self, chunk: &[u8]) {
        let mut output = lock(&self.output);
        output.buffer.extend_from_slice(chunk);
        if output.buffer.len() > SESSION_BUFFER_LIMIT {
            let excess = output.buffer.len() - SESSION_BUFFER_LIMIT;
            let head = SESSION_BUFFER_LIMIT / 2;
            output.buffer.drain(head..head + excess);
            output.dropped_bytes = output.dropped_bytes.saturating_add(excess);
        }
        drop(output);
        self.output_cond.notify_all();
    }

    pub(super) fn mark_closed(&self) {
        let mut output = lock(&self.output);
        output.readers -= 1;
        output.closed = output.readers == 0;
        drop(output);
        self.output_cond.notify_all();
    }

    pub(super) fn mark_exited(&self, exit_code: Option<i32>) {
        let mut output = lock(&self.output);
        output.exited = true;
        output.exited_at = Some(Instant::now());
        output.exit_code = exit_code;
        drop(output);
        self.output_cond.notify_all();
    }

    pub(super) fn drain_expired(&self) -> bool {
        lock(&self.output)
            .exited_at
            .is_some_and(|at| at.elapsed() >= EXIT_DRAIN_GRACE)
    }

    pub(super) fn kill(&self) {
        self.control.kill();
    }
}

pub(super) type SessionMap = HashMap<i64, Arc<ExecSession>>;

pub(super) fn sessions() -> &'static Mutex<SessionMap> {
    static SESSIONS: OnceLock<Mutex<SessionMap>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn owned_session(
    session_id: i64,
    context: &ToolModuleInvocationContext,
) -> Result<Arc<ExecSession>> {
    let sessions = lock(sessions());
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| unknown_session(session_id))?;
    check_owner(session_id, session, context)?;
    Ok(session.clone())
}

pub(super) fn recheck_owned_session(
    session_id: i64,
    session: &Arc<ExecSession>,
    context: &ToolModuleInvocationContext,
) -> Result<()> {
    let sessions = lock(sessions());
    let current = sessions
        .get(&session_id)
        .filter(|current| Arc::ptr_eq(current, session))
        .ok_or_else(|| unknown_session(session_id))?;
    check_owner(session_id, current, context)?;
    current.touch();
    Ok(())
}

fn unknown_session(session_id: i64) -> anyhow::Error {
    anyhow!("unknown exec session {session_id}; the process may have already exited")
}

fn check_owner(
    session_id: i64,
    session: &ExecSession,
    context: &ToolModuleInvocationContext,
) -> Result<()> {
    if !session.owner.matches(context) {
        anyhow::bail!(
            "exec session {session_id} is not owned by the current execution context/workspace"
        );
    }
    Ok(())
}

pub(super) fn ensure_session_janitor() -> Result<()> {
    static JANITOR: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    match JANITOR.get_or_init(|| {
        std::thread::Builder::new()
            .name("proteus-exec-session-janitor".to_owned())
            .spawn(|| {
                loop {
                    std::thread::sleep(SESSION_JANITOR_INTERVAL);
                    prune_expired_sessions(Instant::now(), SESSION_MAX_IDLE);
                }
            })
            .map(|_| ())
            .map_err(|error| format!("failed to spawn exec session janitor: {error}"))
    }) {
        Ok(()) => Ok(()),
        Err(message) => Err(anyhow!(message.clone())),
    }
}

pub(super) static NEXT_SESSION_ID: AtomicI64 = AtomicI64::new(1001);

/// Mutex-и делят только потоки этого модуля; после паники внутри guard'а
/// данные всё ещё согласованны настолько, насколько это возможно — работаем
/// дальше вместо каскадного отказа tool'а.
pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reference lifecycle policy: при заполнении store первыми выкидываются
/// завершённые сессии, затем самая давно не использованная (её процесс
/// убивается). Модель при обращении к вытесненной сессии получает
/// "unknown exec session".
pub(super) fn prune_session_if_needed(sessions: &mut SessionMap) -> Result<()> {
    if sessions.len() < MAX_SESSIONS {
        return Ok(());
    }
    let mut meta: Vec<(i64, Instant, bool)> = sessions
        .iter()
        .map(|(id, session)| (*id, *lock(&session.last_used), lock(&session.output).exited))
        .collect();
    while let Some(victim_id) = session_to_prune(&meta) {
        let victim = sessions[&victim_id].clone();
        if let Some(_interaction) = victim.try_lock_interaction() {
            sessions.remove(&victim_id);
            victim.kill();
            return Ok(());
        }
        meta.retain(|(id, _, _)| *id != victim_id);
    }
    anyhow::bail!("exec session limit ({MAX_SESSIONS}) reached; all sessions are busy")
}

/// Admission and spawn share the store lock: concurrent launches cannot exceed
/// the cap, and rejected launches never execute their command.
pub(super) fn register_session(
    sessions: &mut SessionMap,
    spawn: impl FnOnce() -> Result<Arc<ExecSession>>,
) -> Result<(i64, Arc<ExecSession>)> {
    prune_session_if_needed(sessions)?;
    let session = spawn()?;
    let session_id = NEXT_SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    sessions.insert(session_id, Arc::clone(&session));
    Ok((session_id, session))
}

pub(super) fn prune_expired_sessions(now: Instant, max_idle: Duration) {
    let victims = {
        let mut sessions = lock(sessions());
        let meta = sessions
            .iter()
            .map(|(id, session)| (*id, *lock(&session.last_used), lock(&session.output).exited))
            .collect::<Vec<_>>();
        let mut victims = Vec::new();
        for id in expired_session_ids(&meta, now, max_idle) {
            let candidate = sessions[&id].clone();
            if let Some(_interaction) = candidate.try_lock_interaction() {
                victims.push(sessions.remove(&id).expect("selected session"));
            }
        }
        victims
    };
    for victim in victims {
        victim.kill();
    }
}

pub(super) fn expired_session_ids(
    meta: &[(i64, Instant, bool)],
    now: Instant,
    max_idle: Duration,
) -> Vec<i64> {
    meta.iter()
        .filter(|(_, last_used, _)| now.saturating_duration_since(*last_used) >= max_idle)
        .map(|(id, _, _)| *id)
        .collect()
}

pub(super) fn terminate_session(session_id: i64, session: &ExecSession) {
    lock(sessions()).remove(&session_id);
    session.kill();
}

/// Чистая политика выбора жертвы: сначала exited, затем самый старый
/// `last_used`; id — детерминированный tiebreaker.
pub(super) fn session_to_prune(meta: &[(i64, Instant, bool)]) -> Option<i64> {
    meta.iter()
        .min_by_key(|(id, last_used, exited)| (!exited, *last_used, *id))
        .map(|(id, _, _)| *id)
}
