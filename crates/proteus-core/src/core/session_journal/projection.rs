use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use proteus_contracts::{
    domain::{CallId, ExchangeId, ExecutionId, PartId, RecordId, SessionId, ThreadId, TurnId},
    model_standard::{CanonicalMessage, CanonicalPart, PartScope},
};

use super::types::{
    HistoryMutationKind, JOURNAL_SCHEMA_VERSION, JournalEntry, JournalRecord, ToolCallRecordPhase,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct JournalValidationState {
    history: Vec<CanonicalMessage>,
    history_revision: u64,
    record_ids: HashSet<RecordId>,
    known_parts: HashMap<PartId, CanonicalPart>,
    opened_turns: HashMap<TurnId, TurnLifecycle>,
    execution_turns: HashMap<ExecutionId, TurnId>,
    detached_executions: HashSet<ExecutionId>,
    settled_turns: HashSet<TurnId>,
    model_requests: HashMap<ExchangeId, ExecutionFactOwner>,
    model_responses: HashSet<ExchangeId>,
    tool_calls: HashMap<CallId, ToolLifecycle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TurnLifecycle {
    root_thread_id: ThreadId,
    execution_id: ExecutionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExecutionFactOwner {
    execution_id: ExecutionId,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Default)]
struct ToolLifecycle {
    owner: Option<ExecutionFactOwner>,
    call: Option<proteus_contracts::domain::ToolCall>,
    approval_requested: bool,
    resolved: bool,
    result: bool,
}

impl JournalValidationState {
    pub(crate) fn apply(&mut self, record: &JournalRecord) -> Result<()> {
        if !self.record_ids.insert(record.record_id) {
            bail!("duplicate journal record_id {}", record.record_id);
        }

        match &record.entry {
            JournalEntry::TurnOpened(opened) => {
                let turn_id = required_turn_id(record)?;
                let thread_id = required_thread_id(record)?;
                let execution_id = required_execution_id(record)?;
                if self
                    .opened_turns
                    .insert(
                        turn_id,
                        TurnLifecycle {
                            root_thread_id: thread_id,
                            execution_id,
                        },
                    )
                    .is_some()
                {
                    bail!("turn {turn_id} was opened more than once");
                }
                if self.detached_executions.contains(&execution_id) {
                    bail!(
                        "detached execution {execution_id} cannot later be bound to turn {turn_id}"
                    );
                }
                if let Some(previous_turn) = self.execution_turns.insert(execution_id, turn_id) {
                    bail!(
                        "execution {execution_id} was already bound to turn {previous_turn}, cannot open turn {turn_id}"
                    );
                }
                if opened.base_history_revision != self.history_revision {
                    bail!(
                        "turn {turn_id} opened at history revision {}, current revision is {}",
                        opened.base_history_revision,
                        self.history_revision
                    );
                }
            }
            JournalEntry::HistoryMutated(mutation) => {
                reject_execution_id(record)?;
                if record.turn_id.is_some() {
                    self.require_root_turn(record)?;
                }
                if mutation.previous_revision != self.history_revision {
                    bail!(
                        "history revision mismatch: record expects {}, current revision is {}",
                        mutation.previous_revision,
                        self.history_revision
                    );
                }
                if mutation.new_revision != mutation.previous_revision.saturating_add(1) {
                    bail!(
                        "history revision must increment by one: {} -> {}",
                        mutation.previous_revision,
                        mutation.new_revision
                    );
                }
                validate_conversation_messages(&mutation.messages)?;
                self.validate_part_id_stability(&mutation.messages)?;
                match mutation.mutation {
                    HistoryMutationKind::Append => {
                        if mutation.messages.is_empty() {
                            bail!("history append must contain at least one message");
                        }
                        if mutation.compaction.is_some() {
                            bail!("history append cannot carry a compaction report");
                        }
                        self.history.extend(mutation.messages.iter().cloned());
                    }
                    HistoryMutationKind::Replace => {
                        if mutation
                            .compaction
                            .as_ref()
                            .is_some_and(|report| !report.changed)
                        {
                            bail!("history replacement compaction report must be changed=true");
                        }
                        self.history = mutation.messages.clone();
                    }
                }
                validate_active_history_ids(&self.history)?;
                self.history_revision = mutation.new_revision;
            }
            JournalEntry::ModelRequestRecorded(request) => {
                let owner = self.require_execution_fact(record)?;
                self.validate_part_id_stability(&request.request.messages)?;
                if self
                    .model_requests
                    .insert(request.exchange_id, owner)
                    .is_some()
                {
                    bail!("duplicate model exchange request {}", request.exchange_id);
                }
            }
            JournalEntry::ModelResponseRecorded(response) => {
                let owner = self.require_execution_fact(record)?;
                let Some(request_owner) = self.model_requests.get(&response.exchange_id) else {
                    bail!(
                        "model response {} has no preceding request",
                        response.exchange_id
                    );
                };
                if request_owner != &owner {
                    bail!(
                        "model response {} changed lifecycle owner",
                        response.exchange_id
                    );
                }
                if !self.model_responses.insert(response.exchange_id) {
                    bail!("duplicate model exchange response {}", response.exchange_id);
                }
                if let super::types::ModelResponseOutcome::Response { response } = &response.outcome
                {
                    self.validate_part_id_stability(std::slice::from_ref(&response.message))?;
                }
            }
            JournalEntry::ToolCallRecorded(tool) => {
                let owner = self.require_execution_fact(record)?;
                let lifecycle = self.tool_calls.entry(tool.call.id.clone()).or_default();
                match &tool.phase {
                    ToolCallRecordPhase::Requested => {
                        if lifecycle.call.is_some() {
                            bail!("tool call {} was requested more than once", tool.call.id);
                        }
                        lifecycle.owner = Some(owner);
                        lifecycle.call = Some(tool.call.clone());
                    }
                    ToolCallRecordPhase::ApprovalRequested { .. } => {
                        let Some(requested) = lifecycle.call.as_ref() else {
                            bail!(
                                "tool call {} requested approval before it was recorded",
                                tool.call.id
                            );
                        };
                        if requested != &tool.call {
                            bail!("tool call {} changed before approval request", tool.call.id);
                        }
                        if lifecycle.owner != Some(owner) {
                            bail!("tool call {} changed lifecycle owner", tool.call.id);
                        }
                        if lifecycle.approval_requested {
                            bail!(
                                "tool call {} requested approval more than once",
                                tool.call.id
                            );
                        }
                        if lifecycle.resolved {
                            bail!(
                                "tool call {} requested approval after resolution",
                                tool.call.id
                            );
                        }
                        lifecycle.approval_requested = true;
                    }
                    ToolCallRecordPhase::Resolved { resolution } => {
                        let Some(requested) = lifecycle.call.as_ref() else {
                            bail!(
                                "tool call {} resolved before it was requested",
                                tool.call.id
                            );
                        };
                        if requested != &tool.call {
                            bail!(
                                "tool call {} changed between request and resolution",
                                tool.call.id
                            );
                        }
                        if lifecycle.owner != Some(owner) {
                            bail!("tool call {} changed lifecycle owner", tool.call.id);
                        }
                        if lifecycle.resolved {
                            bail!("tool call {} was resolved more than once", tool.call.id);
                        }
                        if resolution.requested_approval() != lifecycle.approval_requested {
                            bail!(
                                "tool call {} approval lifecycle does not match resolution {:?}",
                                tool.call.id,
                                resolution
                            );
                        }
                        lifecycle.resolved = true;
                    }
                }
            }
            JournalEntry::ToolResultRecorded(tool) => {
                let owner = self.require_execution_fact(record)?;
                let Some(lifecycle) = self.tool_calls.get_mut(&tool.result.call_id) else {
                    bail!("tool result {} has no preceding call", tool.result.call_id);
                };
                if !lifecycle.resolved {
                    bail!(
                        "tool result {} precedes call resolution",
                        tool.result.call_id
                    );
                }
                if lifecycle.owner != Some(owner) {
                    bail!(
                        "tool result {} changed lifecycle owner",
                        tool.result.call_id
                    );
                }
                if lifecycle.result {
                    bail!("duplicate tool result {}", tool.result.call_id);
                }
                lifecycle.result = true;
            }
            JournalEntry::TurnSettled(_) => {
                reject_execution_id(record)?;
                let turn_id = self.require_root_turn(record)?;
                if !self.settled_turns.insert(turn_id) {
                    bail!("turn {turn_id} settled more than once");
                }
            }
        }
        Ok(())
    }

    pub(crate) fn history_revision(&self) -> u64 {
        self.history_revision
    }

    fn validate_part_id_stability(&mut self, messages: &[CanonicalMessage]) -> Result<()> {
        for message in messages {
            for part in &message.parts {
                if let Some(previous) = self.known_parts.get(&part.part_id)
                    && previous != part
                {
                    bail!(
                        "canonical part {} changed after it was recorded",
                        part.part_id
                    );
                }
                self.known_parts.insert(part.part_id, part.clone());
            }
        }
        Ok(())
    }

    fn require_execution_fact(&mut self, record: &JournalRecord) -> Result<ExecutionFactOwner> {
        let execution_id = required_execution_id(record)?;
        match (record.thread_id, record.turn_id) {
            (None, None) => {
                if let Some(turn_id) = self.execution_turns.get(&execution_id) {
                    bail!(
                        "execution {execution_id} is bound to turn {turn_id} and cannot emit detached facts"
                    );
                }
                self.detached_executions.insert(execution_id);
                Ok(ExecutionFactOwner {
                    execution_id,
                    thread_id: None,
                    turn_id: None,
                })
            }
            (Some(thread_id), Some(turn_id)) => {
                let Some(turn) = self.opened_turns.get(&turn_id) else {
                    bail!(
                        "journal record {} ({:?}) attributes execution {execution_id} to turn {turn_id} before it was opened",
                        record.record_id,
                        record.entry.kind()
                    );
                };
                if turn.execution_id != execution_id {
                    bail!(
                        "journal record {} ({:?}) maps turn {turn_id} to execution {execution_id}, expected {}",
                        record.record_id,
                        record.entry.kind(),
                        turn.execution_id
                    );
                }
                if self.settled_turns.contains(&turn_id) && turn.root_thread_id == thread_id {
                    bail!(
                        "journal record {} ({:?}) references settled root turn {turn_id}",
                        record.record_id,
                        record.entry.kind()
                    );
                }
                Ok(ExecutionFactOwner {
                    execution_id,
                    thread_id: Some(thread_id),
                    turn_id: Some(turn_id),
                })
            }
            _ => bail!(
                "journal record {} ({:?}) must provide both thread_id and turn_id for agent attribution, or neither for detached execution",
                record.record_id,
                record.entry.kind()
            ),
        }
    }

    fn require_root_turn(&self, record: &JournalRecord) -> Result<TurnId> {
        let turn_id = required_turn_id(record)?;
        let Some(turn) = self.opened_turns.get(&turn_id) else {
            bail!("turn {turn_id} settled or mutated history before it was opened");
        };
        if Some(turn.root_thread_id) != record.thread_id {
            bail!("turn {turn_id} root lifecycle changed thread_id");
        }
        if self.settled_turns.contains(&turn_id) {
            bail!("turn {turn_id} was already settled");
        }
        Ok(turn_id)
    }
}

#[derive(Debug, Clone)]
pub struct JournalProjection {
    pub records: Vec<JournalRecord>,
    pub history: Vec<CanonicalMessage>,
    pub history_revision: u64,
    pub interrupted_model_exchanges: Vec<ExchangeId>,
    pub unresolved_tool_calls: Vec<CallId>,
    pub unsettled_turns: Vec<TurnId>,
}

impl JournalProjection {
    pub fn build(session_id: SessionId, records: Vec<JournalRecord>) -> Result<Self> {
        let mut state = JournalValidationState::default();
        let mut expected_seq = 1_u64;
        for record in &records {
            if record.schema_version != JOURNAL_SCHEMA_VERSION {
                bail!(
                    "unsupported journal schema_version {} in record {}; expected {}",
                    record.schema_version,
                    record.record_id,
                    JOURNAL_SCHEMA_VERSION
                );
            }
            if record.session_id != session_id {
                bail!(
                    "journal record {} belongs to session {}, expected {}",
                    record.record_id,
                    record.session_id,
                    session_id
                );
            }
            if record.session_seq != expected_seq {
                bail!(
                    "journal sequence mismatch: expected {}, found {}",
                    expected_seq,
                    record.session_seq
                );
            }
            expected_seq = expected_seq.saturating_add(1);
            state.apply(record)?;
        }

        let mut interrupted_model_exchanges = state
            .model_requests
            .keys()
            .filter(|exchange_id| !state.model_responses.contains(exchange_id))
            .copied()
            .collect::<Vec<_>>();
        interrupted_model_exchanges.sort_unstable();
        let mut unresolved_tool_calls = state
            .tool_calls
            .iter()
            .filter(|(_, lifecycle)| !lifecycle.result)
            .map(|(call_id, _)| call_id.clone())
            .collect::<Vec<_>>();
        unresolved_tool_calls.sort();
        let mut unsettled_turns = state
            .opened_turns
            .keys()
            .filter(|turn_id| !state.settled_turns.contains(turn_id))
            .copied()
            .collect::<Vec<_>>();
        unsettled_turns.sort_unstable();

        Ok(Self {
            records,
            history: state.history,
            history_revision: state.history_revision,
            interrupted_model_exchanges,
            unresolved_tool_calls,
            unsettled_turns,
        })
    }
}

fn required_turn_id(record: &JournalRecord) -> Result<TurnId> {
    record.turn_id.ok_or_else(|| {
        anyhow::anyhow!(
            "journal record {} ({:?}) requires turn_id",
            record.record_id,
            record.entry.kind()
        )
    })
}

fn required_thread_id(record: &JournalRecord) -> Result<ThreadId> {
    record.thread_id.ok_or_else(|| {
        anyhow::anyhow!(
            "journal record {} ({:?}) requires thread_id",
            record.record_id,
            record.entry.kind()
        )
    })
}

fn required_execution_id(record: &JournalRecord) -> Result<ExecutionId> {
    record.execution_id.ok_or_else(|| {
        anyhow::anyhow!(
            "journal record {} ({:?}) requires execution_id",
            record.record_id,
            record.entry.kind()
        )
    })
}

fn reject_execution_id(record: &JournalRecord) -> Result<()> {
    if record.execution_id.is_some() {
        bail!(
            "journal record {} ({:?}) is a chat/session lifecycle fact and must not carry execution_id",
            record.record_id,
            record.entry.kind()
        );
    }
    Ok(())
}

fn validate_conversation_messages(messages: &[CanonicalMessage]) -> Result<()> {
    for message in messages {
        for part in &message.parts {
            if part.scope != PartScope::Conversation {
                bail!(
                    "history message {} contains non-conversation part {} with scope {:?}",
                    message.id,
                    part.part_id,
                    part.scope
                );
            }
        }
    }
    Ok(())
}

fn validate_active_history_ids(messages: &[CanonicalMessage]) -> Result<()> {
    let mut message_ids = HashSet::new();
    let mut part_ids = HashSet::new();
    for message in messages {
        if !message_ids.insert(message.id) {
            bail!(
                "active history contains duplicate message id {}",
                message.id
            );
        }
        for part in &message.parts {
            if !part_ids.insert(part.part_id) {
                bail!(
                    "active history contains duplicate part id {} at message {}",
                    part.part_id,
                    message.id,
                );
            }
        }
    }
    Ok(())
}
