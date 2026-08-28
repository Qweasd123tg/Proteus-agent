use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use tokio::sync::Notify;

use crate::{
    contracts::{
        AgentAddress, AgentControlHandle, AgentControlResult, AgentControlToolHost,
        AgentLifecycleStatus, AgentRecordSnapshot,
    },
    domain::{SessionId, ThreadId},
};

const MAX_SESSIONS: usize = 64;
const MAX_AGENTS_PER_SESSION: usize = 64;
const MAX_COMPLETIONS_PER_WAIT: usize = 8;
pub(super) const MAX_OUTSTANDING_COMPLETIONS: usize = 64;
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
    completions: VecDeque<AgentRecordSnapshot>,
}

struct AgentRecord {
    seq: u64,
    task_name: String,
    role: String,
    handle: Option<AgentControlHandle>,
    owner: Option<Arc<dyn AgentControlToolHost>>,
    interrupt_requested: bool,
    generation: u64,
    reserved_generation: Option<u64>,
    outcome: Option<AgentOutcome>,
}

#[derive(Clone)]
enum AgentOutcome {
    Result {
        status: AgentLifecycleStatus,
        child_thread_id: Option<ThreadId>,
        summary: String,
    },
    Error(String),
}

pub(super) struct AgentReservation {
    pub path: String,
    pub generation: u64,
}

pub(super) struct RunningAgent {
    pub path: String,
    pub owner: Arc<dyn AgentControlToolHost>,
    pub handle: AgentControlHandle,
}

pub(super) struct IdleFollowup {
    pub path: String,
    pub task_name: String,
    pub role: String,
    pub task_id: String,
    pub generation: u64,
}

pub(super) enum FollowupRequest {
    Running(RunningAgent),
    Idle(IdleFollowup),
}

pub(super) struct InterruptRequest {
    pub path: String,
    pub owned_handle: Option<(Arc<dyn AgentControlToolHost>, AgentControlHandle)>,
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
    ) -> Result<AgentReservation> {
        let path = AgentAddress::child(task_name)?.to_string();
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
        session.ensure_completion_capacity()?;
        session.agents.insert(
            path.clone(),
            AgentRecord {
                seq,
                task_name: task_name.to_owned(),
                role: role.to_owned(),
                handle: None,
                owner: None,
                interrupt_requested: false,
                generation: 1,
                reserved_generation: None,
                outcome: None,
            },
        );
        Ok(AgentReservation {
            path,
            generation: 1,
        })
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
        generation: u64,
        handle: AgentControlHandle,
        owner: Arc<dyn AgentControlToolHost>,
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
        if record.generation != generation || record.reserved_generation.is_some() {
            bail!("collaboration agent generation changed before attach");
        }
        record.handle = Some(handle);
        record.owner = Some(owner);
        Ok(record.interrupt_requested)
    }

    pub(super) fn complete(
        &self,
        session_id: SessionId,
        path: &str,
        generation: u64,
        result: Result<AgentControlResult>,
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
            if record.generation != generation || record.outcome.is_some() {
                return;
            }
            record.outcome = Some(match result {
                Ok(result) => compact_result(result),
                Err(error) => AgentOutcome::Error(truncate_utf8(
                    format!("{error:#}"),
                    MAX_RETAINED_ERROR_BYTES,
                )),
            });
            // Terminal records retain only bounded presentation data. The
            // originating host owns a full AgentWorkflowContext/registry snapshot
            // and is needed only while the child is addressable.
            record.owner = None;
            record.handle = None;
            record.interrupt_requested = false;
            let completion = view(path, record, true);
            session.completions.push_back(completion);
            session.notify.clone()
        };
        notify.notify_waiters();
    }

    pub(super) fn list(
        &self,
        session_id: SessionId,
        path_prefix: Option<&str>,
    ) -> Result<Vec<AgentRecordSnapshot>> {
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

    pub(super) fn drain_completions(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<AgentRecordSnapshot>> {
        let mut state = self.lock()?;
        let Some(session) = state.sessions.get_mut(&session_id) else {
            return Ok(Vec::new());
        };
        let count = session.completions.len().min(MAX_COMPLETIONS_PER_WAIT);
        Ok(session.completions.drain(..count).collect())
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

    pub(super) fn running_agent(
        &self,
        session_id: SessionId,
        target: &str,
    ) -> Result<RunningAgent> {
        let path = normalize_target(target)?;
        let state = self.lock()?;
        let record = state
            .sessions
            .get(&session_id)
            .and_then(|session| session.agents.get(&path))
            .ok_or_else(|| anyhow!("unknown collaboration agent '{path}' in this session"))?;
        if record.outcome.is_some() && record.reserved_generation.is_none() {
            bail!("collaboration agent '{path}' is idle; use followup_task to start another turn");
        }
        let (owner, handle) = record
            .owner
            .clone()
            .zip(record.handle.clone())
            .ok_or_else(|| anyhow!("collaboration agent '{path}' is still starting"))?;
        Ok(RunningAgent {
            path,
            owner,
            handle,
        })
    }

    pub(super) fn begin_followup(
        &self,
        session_id: SessionId,
        target: &str,
    ) -> Result<FollowupRequest> {
        let path = normalize_target(target)?;
        let mut state = self.lock()?;
        let session = state
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| anyhow!("unknown collaboration agent '{path}' in this session"))?;

        if let Some((owner, handle)) = session
            .agents
            .get(&path)
            .and_then(|record| record.owner.clone().zip(record.handle.clone()))
        {
            return Ok(FollowupRequest::Running(RunningAgent {
                path,
                owner,
                handle,
            }));
        }
        session.ensure_completion_capacity()?;
        let record = session
            .agents
            .get_mut(&path)
            .ok_or_else(|| anyhow!("unknown collaboration agent '{path}' in this session"))?;
        if record.reserved_generation.is_some() {
            bail!("collaboration agent '{path}' already has a follow-up starting");
        }
        let task_id = match &record.outcome {
            Some(AgentOutcome::Result {
                child_thread_id: Some(task_id),
                ..
            }) => task_id.clone(),
            Some(AgentOutcome::Result { .. }) => {
                bail!("collaboration agent '{path}' has no resumable task id")
            }
            Some(AgentOutcome::Error(_)) => {
                bail!("collaboration agent '{path}' errored and cannot be resumed")
            }
            None => bail!("collaboration agent '{path}' is still starting"),
        };
        let generation = record.generation.wrapping_add(1);
        record.reserved_generation = Some(generation);
        Ok(FollowupRequest::Idle(IdleFollowup {
            path,
            task_name: record.task_name.clone(),
            role: record.role.clone(),
            task_id: task_id.to_string(),
            generation,
        }))
    }

    pub(super) fn attach_followup(
        &self,
        session_id: SessionId,
        followup: &IdleFollowup,
        handle: AgentControlHandle,
        owner: Arc<dyn AgentControlToolHost>,
    ) -> Result<bool> {
        let mut state = self.lock()?;
        let record = state
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.agents.get_mut(&followup.path))
            .ok_or_else(|| anyhow!("collaboration follow-up reservation was lost"))?;
        if record.reserved_generation != Some(followup.generation)
            || record.handle.is_some()
            || record.outcome.is_none()
        {
            bail!("collaboration follow-up generation changed before attach");
        }
        let interrupt_requested = record.interrupt_requested;
        record.generation = followup.generation;
        record.reserved_generation = None;
        record.handle = Some(handle);
        record.owner = Some(owner);
        record.outcome = None;
        record.interrupt_requested = false;
        Ok(interrupt_requested)
    }

    pub(super) fn abort_followup(&self, session_id: SessionId, path: &str, generation: u64) {
        if let Ok(mut state) = self.lock()
            && let Some(record) = state
                .sessions
                .get_mut(&session_id)
                .and_then(|session| session.agents.get_mut(path))
            && record.reserved_generation == Some(generation)
        {
            record.reserved_generation = None;
            record.interrupt_requested = false;
        }
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
        let terminal = record.outcome.is_some() && record.reserved_generation.is_none();
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
        self.agents
            .values()
            .all(|record| record.outcome.is_some() && record.reserved_generation.is_none())
    }

    fn prune_for_spawn(&mut self) -> Result<()> {
        while self.agents.len() >= MAX_AGENTS_PER_SESSION {
            let evict = self
                .agents
                .iter()
                .filter(|(_, record)| {
                    record.outcome.is_some() && record.reserved_generation.is_none()
                })
                .min_by_key(|(_, record)| record.seq)
                .map(|(path, _)| path.clone());
            match evict {
                Some(path) => {
                    self.agents.remove(&path);
                    self.completions
                        .retain(|queued| queued.path.as_str() != path);
                }
                None => bail!(
                    "collaboration agent capacity reached ({MAX_AGENTS_PER_SESSION}); active agents are not evicted"
                ),
            }
        }
        Ok(())
    }

    fn ensure_completion_capacity(&self) -> Result<()> {
        let active = self
            .agents
            .values()
            .filter(|record| record.outcome.is_none() || record.reserved_generation.is_some())
            .count();
        if self.completions.len().saturating_add(active) >= MAX_OUTSTANDING_COMPLETIONS {
            bail!(
                "collaboration completion capacity reached ({MAX_OUTSTANDING_COMPLETIONS}); drain updates with wait_agent before starting more work"
            );
        }
        Ok(())
    }
}

fn view(path: &str, record: &AgentRecord, include_payload: bool) -> AgentRecordSnapshot {
    let (status, child_thread_id, summary, error) = match &record.outcome {
        _ if record.reserved_generation.is_some() => {
            (AgentLifecycleStatus::Starting, None, None, None)
        }
        Some(AgentOutcome::Result {
            status,
            child_thread_id,
            summary,
        }) => (
            *status,
            *child_thread_id,
            include_payload.then(|| summary.clone()),
            None,
        ),
        Some(AgentOutcome::Error(error)) => (
            AgentLifecycleStatus::Errored,
            None,
            None,
            include_payload.then(|| error.clone()),
        ),
        None if record.handle.is_some() => (AgentLifecycleStatus::Running, None, None, None),
        None => (AgentLifecycleStatus::Starting, None, None, None),
    };
    AgentRecordSnapshot {
        path: AgentAddress::parse(path).expect("control stores canonical agent paths"),
        task_name: record.task_name.clone(),
        agent_type: record.role.clone(),
        generation: record.reserved_generation.unwrap_or(record.generation),
        status,
        child_thread_id,
        summary,
        error,
    }
}

fn compact_result(result: AgentControlResult) -> AgentOutcome {
    let child_thread_id = result
        .metadata
        .get("resumable")
        .and_then(Value::as_bool)
        .filter(|resumable| *resumable)
        .and(result.child_thread_id);
    AgentOutcome::Result {
        status: result.status,
        child_thread_id,
        summary: truncate_utf8(result.summary, MAX_RETAINED_SUMMARY_BYTES),
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

fn normalize_target(target: &str) -> Result<String> {
    let address = if target.starts_with('/') {
        AgentAddress::parse(target)?
    } else {
        AgentAddress::child(target)?
    };
    if address == AgentAddress::root() {
        bail!("collaboration target must be a child address");
    }
    Ok(address.to_string())
}

fn normalize_prefix(prefix: Option<&str>) -> Result<String> {
    match prefix {
        None | Some("") | Some("/root") | Some("/root/") => Ok("/root/".to_owned()),
        Some(prefix) => normalize_target(prefix),
    }
}
