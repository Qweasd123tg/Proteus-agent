use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    core::{
        HistoryMutated, HistoryMutationKind, JournalEntry, JournalRecord, ModelResponseOutcome,
        SessionConfigSnapshot, SessionStore, ToolCallRecordPhase, TurnOpened, TurnSettled,
        normalize_session_dir_path,
    },
    domain::{
        CallId, ContextBundle, ContextChunk, ExchangeId, HistoryCompactionReport, SessionId,
        ThreadId, ToolCall, ToolCallResolution, ToolResult, TurnId,
    },
    model_standard::{CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole},
};

use super::WorkflowReplayOptions;

#[derive(Debug, Clone)]
pub(super) struct WorkflowReplayFixture {
    pub journal_path: std::path::PathBuf,
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub opened: TurnOpened,
    pub snapshot: SessionConfigSnapshot,
    pub initial_history: Vec<CanonicalMessage>,
    pub final_history: Vec<CanonicalMessage>,
    pub context: ContextBundle,
    pub exchanges: Vec<RecordedModelExchange>,
    pub tools: Vec<RecordedToolInvocation>,
    pub compactions: Vec<HistoryCompactionReport>,
    pub settlement: TurnSettled,
}

#[derive(Debug, Clone)]
pub(super) struct RecordedModelExchange {
    pub exchange_id: ExchangeId,
    pub request: CanonicalModelRequest,
    pub outcome: ModelResponseOutcome,
}

#[derive(Debug, Clone)]
pub(super) struct RecordedToolInvocation {
    pub call: ToolCall,
    pub approval_reason: Option<String>,
    pub resolution: ToolCallResolution,
    pub result: ToolResult,
}

#[derive(Debug)]
struct PendingToolInvocation {
    call: ToolCall,
    approval_reason: Option<String>,
    resolution: Option<ToolCallResolution>,
    result: Option<ToolResult>,
}

pub(super) fn load_fixture(
    path: &Path,
    options: WorkflowReplayOptions,
) -> Result<WorkflowReplayFixture> {
    let session_dir = normalize_session_dir_path(path.to_path_buf())?;
    let store = SessionStore::open(session_dir.clone()).with_context(|| {
        format!(
            "failed to open canonical session journal at {}",
            session_dir.display()
        )
    })?;
    let projection = store.load_projection()?;
    let (turn_id, thread_id, opened) = select_turn(&projection.records, options.turn_id)?;
    let snapshot = parse_snapshot(&opened, turn_id)?;
    let (settlement, compactions) =
        select_settlement_and_compactions(&projection.records, turn_id, thread_id)?;
    ensure_replayable_settlement(turn_id, &settlement)?;
    let history = select_history(&projection.records, turn_id, thread_id, &opened)?;
    let exchanges = select_exchanges(&projection.records, turn_id, thread_id)?;
    if exchanges.is_empty() {
        bail!(
            "turn {turn_id} contains no completed root model exchanges; workflow replay needs at least one recorded model outcome"
        );
    }
    let tools = select_tools(&projection.records, turn_id, thread_id)?;
    let context = recorded_context(&exchanges[0].request, &settlement);

    Ok(WorkflowReplayFixture {
        journal_path: store.journal_path(),
        session_id: store.session_id(),
        thread_id,
        turn_id,
        opened,
        snapshot,
        initial_history: history.initial,
        final_history: history.final_history,
        context,
        exchanges,
        tools,
        compactions,
        settlement,
    })
}

fn select_turn(
    records: &[JournalRecord],
    requested_id: Option<TurnId>,
) -> Result<(TurnId, ThreadId, TurnOpened)> {
    let turns = records
        .iter()
        .filter_map(|record| match (&record.entry, record.turn_id) {
            (JournalEntry::TurnOpened(opened), Some(turn_id)) => {
                Some((turn_id, record.thread_id, opened.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if turns.is_empty() {
        bail!("canonical session journal contains no turns");
    }
    let available = turns
        .iter()
        .map(|(turn_id, _, _)| turn_id.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    match requested_id {
        Some(turn_id) => turns
            .into_iter()
            .find(|(candidate, _, _)| *candidate == turn_id)
            .ok_or_else(|| {
                anyhow!("turn {turn_id} was not found; available turn IDs: {available}")
            }),
        None if turns.len() == 1 => turns
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("canonical session journal contains no turns")),
        None => bail!(
            "canonical session journal contains multiple turns; pass --turn-id <id>; available turn IDs: {available}"
        ),
    }
}

fn parse_snapshot(opened: &TurnOpened, turn_id: TurnId) -> Result<SessionConfigSnapshot> {
    let snapshot: Option<SessionConfigSnapshot> =
        serde_json::from_value(opened.config_snapshot.clone()).with_context(|| {
            format!("turn {turn_id} contains an invalid runtime config snapshot")
        })?;
    let snapshot = snapshot.ok_or_else(|| {
        anyhow!(
            "turn {turn_id} has no runtime config snapshot; workflow replay cannot resolve the recorded workflow and policy"
        )
    })?;
    if snapshot.schema_version != 3 {
        bail!(
            "turn {turn_id} uses unsupported config snapshot schema_version {}; expected 3",
            snapshot.schema_version
        );
    }
    Ok(snapshot)
}

fn ensure_replayable_settlement(turn_id: TurnId, settlement: &TurnSettled) -> Result<()> {
    match settlement.status {
        crate::core::TurnSettlementStatus::Success | crate::core::TurnSettlementStatus::Error => {
            Ok(())
        }
        crate::core::TurnSettlementStatus::Canceled => bail!(
            "turn {turn_id} settled as canceled; workflow replay v0 cannot reproduce external cancellation timing; verify durable cancellation through the canonical journal and cold /history"
        ),
        crate::core::TurnSettlementStatus::Timeout => bail!(
            "turn {turn_id} settled as timeout; workflow replay v0 cannot reproduce the runtime-owned timeout boundary; verify durable timeout recovery through the canonical journal and cold /history"
        ),
    }
}

struct SelectedHistory {
    initial: Vec<CanonicalMessage>,
    final_history: Vec<CanonicalMessage>,
}

fn select_history(
    records: &[JournalRecord],
    turn_id: TurnId,
    thread_id: ThreadId,
    opened: &TurnOpened,
) -> Result<SelectedHistory> {
    let mut history = Vec::new();
    let mut revision = 0_u64;
    let mut turn_open = false;
    let mut initial = None;
    let mut final_history = None;

    for record in records {
        if matches!(&record.entry, JournalEntry::TurnOpened(_)) && record.turn_id == Some(turn_id) {
            turn_open = true;
            if revision != opened.base_history_revision {
                bail!(
                    "turn {turn_id} base history revision {} does not match replay fold revision {revision}",
                    opened.base_history_revision
                );
            }
        }

        if let JournalEntry::HistoryMutated(mutation) = &record.entry {
            if turn_open
                && final_history.is_none()
                && record.turn_id.is_some()
                && record.turn_id != Some(turn_id)
            {
                bail!(
                    "turn {turn_id} overlaps another turn's history mutation; concurrent same-session workflow replay is not supported"
                );
            }
            apply_history_mutation(&mut history, &mut revision, mutation)?;
            if record.turn_id == Some(turn_id) && record.thread_id == thread_id {
                if initial.is_none() {
                    if mutation.mutation != HistoryMutationKind::Append {
                        bail!("turn {turn_id} did not begin with a persisted user-message append");
                    }
                    validate_current_user_message(&history, opened, turn_id)?;
                    initial = Some(history.clone());
                } else if mutation.mutation == HistoryMutationKind::Append
                    && mutation
                        .messages
                        .iter()
                        .any(|message| message.role == MessageRole::User)
                {
                    bail!(
                        "turn {turn_id} contains delivered steering/follow-up history; workflow replay v0 does not emulate the root steering decorator"
                    );
                }
            }
        }

        if matches!(&record.entry, JournalEntry::TurnSettled(_))
            && record.turn_id == Some(turn_id)
            && record.thread_id == thread_id
        {
            final_history = Some(history.clone());
            turn_open = false;
        }
    }

    Ok(SelectedHistory {
        initial: initial.ok_or_else(|| {
            anyhow!(
                "turn {turn_id} has no persisted current user message before workflow execution"
            )
        })?,
        final_history: final_history
            .ok_or_else(|| anyhow!("turn {turn_id} has no terminal turn_settled record"))?,
    })
}

fn apply_history_mutation(
    history: &mut Vec<CanonicalMessage>,
    revision: &mut u64,
    mutation: &HistoryMutated,
) -> Result<()> {
    if mutation.previous_revision != *revision {
        bail!(
            "history revision mismatch while selecting workflow replay: expected {}, found {}",
            *revision,
            mutation.previous_revision
        );
    }
    match mutation.mutation {
        HistoryMutationKind::Append => history.extend(mutation.messages.iter().cloned()),
        HistoryMutationKind::Replace => *history = mutation.messages.clone(),
    }
    *revision = mutation.new_revision;
    Ok(())
}

fn validate_current_user_message(
    history: &[CanonicalMessage],
    opened: &TurnOpened,
    turn_id: TurnId,
) -> Result<()> {
    let current = history
        .last()
        .ok_or_else(|| anyhow!("turn {turn_id} persisted an empty history append"))?;
    if current.role != MessageRole::User || message_text(current) != opened.task.text {
        bail!("turn {turn_id} persisted user message does not match turn_opened task text");
    }
    Ok(())
}

fn select_exchanges(
    records: &[JournalRecord],
    turn_id: TurnId,
    thread_id: ThreadId,
) -> Result<Vec<RecordedModelExchange>> {
    struct PendingExchange {
        exchange_id: ExchangeId,
        request: CanonicalModelRequest,
        outcome: Option<ModelResponseOutcome>,
    }

    let mut exchanges = Vec::<PendingExchange>::new();
    let mut positions = HashMap::new();
    for record in records
        .iter()
        .filter(|record| record.turn_id == Some(turn_id) && record.thread_id == thread_id)
    {
        match &record.entry {
            JournalEntry::ModelRequestRecorded(request) => {
                positions.insert(request.exchange_id, exchanges.len());
                exchanges.push(PendingExchange {
                    exchange_id: request.exchange_id,
                    request: request.request.clone(),
                    outcome: None,
                });
            }
            JournalEntry::ModelResponseRecorded(response) => {
                let index = positions
                    .get(&response.exchange_id)
                    .copied()
                    .ok_or_else(|| {
                        anyhow!(
                            "model response {} has no selected root request",
                            response.exchange_id
                        )
                    })?;
                exchanges[index].outcome = Some(response.outcome.clone());
            }
            _ => {}
        }
    }
    exchanges
        .into_iter()
        .map(|exchange| {
            Ok(RecordedModelExchange {
                exchange_id: exchange.exchange_id,
                request: exchange.request,
                outcome: exchange.outcome.ok_or_else(|| {
                    anyhow!(
                        "model exchange {} is incomplete and cannot be used for workflow replay",
                        exchange.exchange_id
                    )
                })?,
            })
        })
        .collect()
}

fn select_tools(
    records: &[JournalRecord],
    turn_id: TurnId,
    thread_id: ThreadId,
) -> Result<Vec<RecordedToolInvocation>> {
    let mut pending = Vec::<PendingToolInvocation>::new();
    let mut positions = HashMap::<CallId, usize>::new();
    for record in records
        .iter()
        .filter(|record| record.turn_id == Some(turn_id) && record.thread_id == thread_id)
    {
        match &record.entry {
            JournalEntry::ToolCallRecorded(recorded) => match &recorded.phase {
                ToolCallRecordPhase::Requested => {
                    positions.insert(recorded.call.id.clone(), pending.len());
                    pending.push(PendingToolInvocation {
                        call: recorded.call.clone(),
                        approval_reason: None,
                        resolution: None,
                        result: None,
                    });
                }
                ToolCallRecordPhase::ApprovalRequested { reason } => {
                    let invocation = pending_tool(&mut pending, &positions, &recorded.call.id)?;
                    invocation.approval_reason = Some(reason.clone());
                }
                ToolCallRecordPhase::Resolved { resolution } => {
                    let invocation = pending_tool(&mut pending, &positions, &recorded.call.id)?;
                    invocation.resolution = Some(resolution.clone());
                }
            },
            JournalEntry::ToolResultRecorded(recorded) => {
                let invocation = pending_tool(&mut pending, &positions, &recorded.result.call_id)?;
                invocation.result = Some(recorded.result.clone());
            }
            _ => {}
        }
    }

    pending
        .into_iter()
        .map(|pending| {
            let call_id = pending.call.id.clone();
            Ok(RecordedToolInvocation {
                call: pending.call,
                approval_reason: pending.approval_reason,
                resolution: pending
                    .resolution
                    .ok_or_else(|| anyhow!("tool call {call_id} has no recorded resolution"))?,
                result: pending
                    .result
                    .ok_or_else(|| anyhow!("tool call {call_id} has no recorded result"))?,
            })
        })
        .collect()
}

fn pending_tool<'a>(
    pending: &'a mut [PendingToolInvocation],
    positions: &HashMap<CallId, usize>,
    call_id: &str,
) -> Result<&'a mut PendingToolInvocation> {
    let index = positions
        .get(call_id)
        .copied()
        .ok_or_else(|| anyhow!("tool lifecycle for {call_id} has no selected root request"))?;
    pending
        .get_mut(index)
        .ok_or_else(|| anyhow!("tool lifecycle index for {call_id} is invalid"))
}

fn select_settlement_and_compactions(
    records: &[JournalRecord],
    turn_id: TurnId,
    thread_id: ThreadId,
) -> Result<(TurnSettled, Vec<HistoryCompactionReport>)> {
    let mut settlement = None;
    let mut compactions = Vec::new();
    for record in records
        .iter()
        .filter(|record| record.turn_id == Some(turn_id) && record.thread_id == thread_id)
    {
        match &record.entry {
            JournalEntry::HistoryMutated(mutation) => {
                if let Some(report) = &mutation.compaction {
                    compactions.push(report.clone());
                }
            }
            JournalEntry::TurnSettled(recorded) => settlement = Some(recorded.clone()),
            _ => {}
        }
    }
    Ok((
        settlement.ok_or_else(|| anyhow!("turn {turn_id} has no terminal turn_settled record"))?,
        compactions,
    ))
}

fn recorded_context(request: &CanonicalModelRequest, settlement: &TurnSettled) -> ContextBundle {
    let mut chunks = request
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match &part.payload {
            ContentPart::Context { chunk } => Some(chunk.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected_chunks = settlement
        .output
        .as_ref()
        .and_then(|output| output.metadata.get("context"))
        .and_then(|context| context.get("chunks"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(chunks.len());
    while chunks.len() < expected_chunks {
        chunks.push(ContextChunk::new(
            "workflow_replay.unavailable",
            "context removed by recorded compaction",
        ));
    }
    let mut bundle = ContextBundle::new(chunks);
    bundle.token_estimate = settlement
        .output
        .as_ref()
        .and_then(|output| output.metadata.get("context"))
        .and_then(|context| context.get("initial_token_estimate"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|tokens| u32::try_from(tokens).ok());
    bundle
}

fn message_text(message: &CanonicalMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match &part.payload {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
