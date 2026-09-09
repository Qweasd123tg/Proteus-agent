//! Session ownership, storage and lifetime.
use super::{
    EXIT_DRAIN_GRACE, MAX_SESSIONS, SESSION_BUFFER_LIMIT, SESSION_JANITOR_INTERVAL,
    SESSION_MAX_IDLE,
};
use crate::sandbox::SandboxKind;
use anyhow::{Result, anyhow};
use proteus_contracts::{
    domain::{ExecutionId, SessionId, ThreadId},
    process_module::ToolModuleInvocationContext,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, atomic::AtomicI64},
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
pub(super) fn prune_session_if_needed(sessions: &mut SessionMap) {
    if sessions.len() < MAX_SESSIONS {
        return;
    }
    let meta: Vec<(i64, Instant, bool)> = sessions
        .iter()
        .map(|(id, session)| (*id, *lock(&session.last_used), lock(&session.output).exited))
        .collect();
    let Some(victim_id) = session_to_prune(&meta) else {
        return;
    };
    if let Some(victim) = sessions.remove(&victim_id) {
        victim.kill();
    }
}

pub(super) fn prune_expired_sessions(now: Instant, max_idle: Duration) {
    let victims = {
        let mut sessions = lock(sessions());
        let meta = sessions
            .iter()
            .map(|(id, session)| (*id, *lock(&session.last_used), lock(&session.output).exited))
            .collect::<Vec<_>>();
        expired_session_ids(&meta, now, max_idle)
            .into_iter()
            .filter_map(|id| sessions.remove(&id))
            .collect::<Vec<_>>()
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
