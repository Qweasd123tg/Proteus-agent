use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use tokio::sync::Notify;

use crate::{
    contracts::{SubagentHandle, SubagentResult, SubagentStatus, SubagentToolHost},
    domain::SessionId,
};

const MAX_SESSIONS: usize = 64;
const MAX_AGENTS_PER_SESSION: usize = 64;
const MAX_COMPLETIONS_PER_WAIT: usize = 8;
const MAX_RETAINED_SUMMARY_BYTES: usize = 16_000;
const MAX_RETAINED_ERROR_BYTES: usize = 4_000;

#[derive(Clone, Default)]
pub(super) struct CollaborationControl {
    inner: Arc<Mutex<ControlState>>,
}

#[derive(Default)]
struct ControlState {
    sessions: HashMap<SessionId, SessionState>,
    next_seq: u64,
}

struct SessionState {
    seq: u64,
    notify: Arc<Notify>,
    agents: BTreeMap<String, AgentRecord>,
    completions: VecDeque<String>,
}

struct AgentRecord {
    seq: u64,
    task_name: String,
    role: String,
    handle: Option<SubagentHandle>,
    owner: Option<Arc<dyn SubagentToolHost>>,
    interrupt_requested: bool,
    outcome: Option<AgentOutcome>,
}

#[derive(Clone)]
enum AgentOutcome {
    Result {
        status: String,
        child_thread_id: Option<String>,
        summary: String,
    },
    Error(String),
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AgentView {
    pub path: String,
    pub task_name: String,
    pub agent_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(super) struct InterruptRequest {
    pub path: String,
    pub owned_handle: Option<(Arc<dyn SubagentToolHost>, SubagentHandle)>,
    pub terminal: bool,
}

impl CollaborationControl {
    pub(super) fn shared() -> Self {
        static CONTROL: OnceLock<CollaborationControl> = OnceLock::new();
        CONTROL.get_or_init(Self::default).clone()
    }

    pub(super) fn reserve(
        &self,
        session_id: SessionId,
        task_name: &str,
        role: &str,
    ) -> Result<String> {
        validate_task_name(task_name)?;
        let path = canonical_path(task_name);
        let mut state = self.lock()?;
        state.ensure_session(session_id)?;
        let seq = state.next_seq;
        state.next_seq = state.next_seq.wrapping_add(1);
        let session = state
            .sessions
            .get_mut(&session_id)
            .expect("session inserted by ensure_session");
        if session.agents.contains_key(&path) {
            bail!("task_name '{task_name}' is already owned by this session");
        }
        session.prune_for_spawn()?;
        session.agents.insert(
            path.clone(),
            AgentRecord {
                seq,
                task_name: task_name.to_owned(),
                role: role.to_owned(),
                handle: None,
                owner: None,
                interrupt_requested: false,
                outcome: None,
            },
        );
        Ok(path)
    }

    pub(super) fn release_reservation(&self, session_id: SessionId, path: &str) {
        if let Ok(mut state) = self.lock()
            && let Some(session) = state.sessions.get_mut(&session_id)
        {
            session.agents.remove(path);
        }
    }

    pub(super) fn attach(
        &self,
        session_id: SessionId,
        path: &str,
        handle: SubagentHandle,
        owner: Arc<dyn SubagentToolHost>,
    ) -> Result<bool> {
        let mut state = self.lock()?;
        let record = state
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.agents.get_mut(path))
            .ok_or_else(|| anyhow!("collaboration spawn reservation was lost"))?;
        if record.handle.is_some() {
            bail!("collaboration agent handle is already attached");
        }
        record.handle = Some(handle);
        record.owner = Some(owner);
        Ok(record.interrupt_requested)
    }

    pub(super) fn complete(
        &self,
        session_id: SessionId,
        path: &str,
        result: Result<SubagentResult>,
    ) {
        let notify = {
            let Ok(mut state) = self.lock() else {
                return;
            };
            let Some(session) = state.sessions.get_mut(&session_id) else {
                return;
            };
            let Some(record) = session.agents.get_mut(path) else {
                return;
            };
            record.outcome = Some(match result {
                Ok(result) => compact_result(result),
                Err(error) => AgentOutcome::Error(truncate_utf8(
                    format!("{error:#}"),
                    MAX_RETAINED_ERROR_BYTES,
                )),
            });
            // Terminal records retain only bounded presentation data. The
            // originating host owns a full RuntimeContext/registry snapshot
            // and is needed only while the child is addressable.
            record.owner = None;
            record.handle = None;
            session.completions.push_back(path.to_owned());
            session.notify.clone()
        };
        notify.notify_waiters();
    }

    pub(super) fn list(
        &self,
        session_id: SessionId,
        path_prefix: Option<&str>,
    ) -> Result<Vec<AgentView>> {
        let prefix = normalize_prefix(path_prefix)?;
        let state = self.lock()?;
        let Some(session) = state.sessions.get(&session_id) else {
            return Ok(Vec::new());
        };
        let root_prefix = prefix == "/root/";
        let mut records = session
            .agents
            .iter()
            .filter(|(path, _)| root_prefix || *path == &prefix)
            .map(|(path, record)| (record.seq, view(path, record, false)))
            .collect::<Vec<_>>();
        records.sort_by_key(|(seq, _)| *seq);
        Ok(records.into_iter().map(|(_, view)| view).collect())
    }

    pub(super) fn has_completions(&self, session_id: SessionId) -> Result<bool> {
        let state = self.lock()?;
        Ok(state
            .sessions
            .get(&session_id)
            .is_some_and(|session| !session.completions.is_empty()))
    }

    pub(super) fn drain_completions(&self, session_id: SessionId) -> Result<Vec<AgentView>> {
        let mut state = self.lock()?;
        let Some(session) = state.sessions.get_mut(&session_id) else {
            return Ok(Vec::new());
        };
        let count = session.completions.len().min(MAX_COMPLETIONS_PER_WAIT);
        let paths = session.completions.drain(..count).collect::<Vec<_>>();
        Ok(paths
            .iter()
            .filter_map(|path| {
                session
                    .agents
                    .get(path)
                    .map(|record| view(path, record, true))
            })
            .collect())
    }

    pub(super) fn session_notify(&self, session_id: SessionId) -> Result<Arc<Notify>> {
        let mut state = self.lock()?;
        state.ensure_session(session_id)?;
        Ok(state
            .sessions
            .get(&session_id)
            .expect("session inserted by ensure_session")
            .notify
            .clone())
    }

    pub(super) fn request_interrupt(
        &self,
        session_id: SessionId,
        target: &str,
    ) -> Result<InterruptRequest> {
        let path = normalize_target(target)?;
        let mut state = self.lock()?;
        let record = state
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.agents.get_mut(&path))
            .ok_or_else(|| anyhow!("unknown collaboration agent '{path}' in this session"))?;
        let terminal = record.outcome.is_some();
        if !terminal {
            record.interrupt_requested = true;
        }
        let owned_handle = record.owner.clone().zip(record.handle.clone());
        Ok(InterruptRequest {
            path,
            owned_handle,
            terminal,
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, ControlState>> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("collaboration control lock poisoned"))
    }
}

impl ControlState {
    fn ensure_session(&mut self, session_id: SessionId) -> Result<()> {
        if self.sessions.contains_key(&session_id) {
            return Ok(());
        }
        while self.sessions.len() >= MAX_SESSIONS {
            let evict = self
                .sessions
                .iter()
                .filter(|(_, session)| session.all_terminal())
                .min_by_key(|(_, session)| session.seq)
                .map(|(id, _)| *id);
            match evict {
                Some(id) => {
                    self.sessions.remove(&id);
                }
                None => bail!(
                    "collaboration session capacity reached ({MAX_SESSIONS}); active sessions are not evicted"
                ),
            }
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.sessions.insert(
            session_id,
            SessionState {
                seq,
                notify: Arc::new(Notify::new()),
                agents: BTreeMap::new(),
                completions: VecDeque::new(),
            },
        );
        Ok(())
    }
}

impl SessionState {
    fn all_terminal(&self) -> bool {
        self.agents.values().all(|record| record.outcome.is_some())
    }

    fn prune_for_spawn(&mut self) -> Result<()> {
        while self.agents.len() >= MAX_AGENTS_PER_SESSION {
            let evict = self
                .agents
                .iter()
                .filter(|(_, record)| record.outcome.is_some())
                .min_by_key(|(_, record)| record.seq)
                .map(|(path, _)| path.clone());
            match evict {
                Some(path) => {
                    self.agents.remove(&path);
                    self.completions.retain(|queued| queued != &path);
                }
                None => bail!(
                    "collaboration agent capacity reached ({MAX_AGENTS_PER_SESSION}); active agents are not evicted"
                ),
            }
        }
        Ok(())
    }
}

fn view(path: &str, record: &AgentRecord, include_payload: bool) -> AgentView {
    let (status, child_thread_id, summary, error) = match &record.outcome {
        Some(AgentOutcome::Result {
            status,
            child_thread_id,
            summary,
        }) => (
            status.clone(),
            child_thread_id.clone(),
            include_payload.then(|| summary.clone()),
            None,
        ),
        Some(AgentOutcome::Error(error)) => (
            "errored".to_owned(),
            None,
            None,
            include_payload.then(|| error.clone()),
        ),
        None if record.handle.is_some() => ("running".to_owned(), None, None, None),
        None => ("starting".to_owned(), None, None, None),
    };
    AgentView {
        path: path.to_owned(),
        task_name: record.task_name.clone(),
        agent_type: record.role.clone(),
        status,
        child_thread_id,
        summary,
        error,
    }
}

fn compact_result(result: SubagentResult) -> AgentOutcome {
    AgentOutcome::Result {
        status: status_label(result.status).to_owned(),
        child_thread_id: result.child_thread_id.map(|id| id.to_string()),
        summary: truncate_utf8(result.summary, MAX_RETAINED_SUMMARY_BYTES),
    }
}

fn status_label(status: SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::Completed => "completed",
        SubagentStatus::MaxIterationsReached => "max_iterations_reached",
        SubagentStatus::TimedOut => "timed_out",
        SubagentStatus::Cancelled => "cancelled",
        SubagentStatus::TokenBudgetExceeded => "token_budget_exceeded",
        _ => "unknown",
    }
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

pub(super) fn validate_task_name(task_name: &str) -> Result<()> {
    if task_name.is_empty()
        || task_name == "root"
        || task_name.len() > 64
        || !task_name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        bail!("task_name must be one [a-z0-9_]+ segment (1-64 bytes) and cannot be 'root'");
    }
    Ok(())
}

fn canonical_path(task_name: &str) -> String {
    format!("/root/{task_name}")
}

fn normalize_target(target: &str) -> Result<String> {
    if let Some(task_name) = target.strip_prefix("/root/") {
        validate_task_name(task_name)?;
        return Ok(canonical_path(task_name));
    }
    validate_task_name(target)?;
    Ok(canonical_path(target))
}

fn normalize_prefix(prefix: Option<&str>) -> Result<String> {
    match prefix {
        None | Some("") | Some("/root") | Some("/root/") => Ok("/root/".to_owned()),
        Some(prefix) => normalize_target(prefix),
    }
}
