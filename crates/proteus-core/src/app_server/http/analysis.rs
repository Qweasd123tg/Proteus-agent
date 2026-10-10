//! Historical inspection never resumes a session or executes a module.
use std::{collections::HashMap, path::PathBuf};

use anyhow::{Result, anyhow};
use proteus_contracts::{app_protocol::analysis::*, domain::TurnId};

use super::{
    HttpAppState,
    sessions::{percent_decode_query_value, required_session_query},
};
use crate::core::{
    JournalEntry, JournalProjection, ModelResponseOutcome, SessionStore, ToolCallRecordPhase,
    TurnSettlementStatus, canonicalize_session_dir_path,
};

pub(super) async fn read(state: &HttpAppState, query: Option<&str>) -> Result<AppSessionAnalysis> {
    let (session_dir, requested) = parse(query)?;
    // A live session writes its directory with the first journal entry; until then its journal is empty.
    if !tokio::fs::try_exists(&session_dir).await?
        && let Some(server) = state.server_for_session_dir(&session_dir).await
    {
        let session_id = server.session_id();
        return project(
            session_id,
            &JournalProjection::build(session_id, Vec::new())?,
            requested,
        );
    }
    read_stored(session_dir, requested).await
}

fn parse(query: Option<&str>) -> Result<(PathBuf, Option<TurnId>)> {
    let session_dir = canonicalize_session_dir_path(required_session_query(query)?)?;
    let mut requested = None;
    for pair in query.unwrap_or_default().split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "turn_id" {
            if requested.is_some() {
                return Err(anyhow!("duplicate turn_id query parameter"));
            }
            requested = Some(percent_decode_query_value(value)?.parse::<TurnId>()?);
        }
    }
    Ok((session_dir, requested))
}

async fn read_stored(
    session_dir: PathBuf,
    requested: Option<TurnId>,
) -> Result<AppSessionAnalysis> {
    tokio::task::spawn_blocking(move || {
        let store = SessionStore::open(session_dir)?;
        let projection = store.load_projection()?;
        project(store.session_id(), &projection, requested)
    })
    .await?
}

fn project(
    session_id: proteus_contracts::domain::SessionId,
    projection: &JournalProjection,
    requested: Option<TurnId>,
) -> Result<AppSessionAnalysis> {
    let mut turns: Vec<AppAnalysisTurn> = Vec::new();
    let mut indices = HashMap::new();
    for record in &projection.records {
        let Some(turn_id) = record.turn_id else {
            continue;
        };
        match &record.entry {
            JournalEntry::TurnOpened(opened) => {
                indices.insert(turn_id, turns.len());
                turns.push(AppAnalysisTurn {
                    turn_id,
                    number: turns.len() + 1,
                    prompt_preview: opened.task.text.chars().take(180).collect(),
                    started_at_ms: record.timestamp_ms,
                    finished_at_ms: None,
                    status: AppAnalysisTurnStatus::Unsettled,
                });
            }
            JournalEntry::TurnSettled(settled) => {
                if let Some(&index) = indices.get(&turn_id) {
                    turns[index].finished_at_ms = Some(record.timestamp_ms);
                    turns[index].status = match settled.status {
                        TurnSettlementStatus::Success => AppAnalysisTurnStatus::Success,
                        TurnSettlementStatus::Error => AppAnalysisTurnStatus::Error,
                        TurnSettlementStatus::Canceled => AppAnalysisTurnStatus::Canceled,
                        TurnSettlementStatus::Timeout => AppAnalysisTurnStatus::Timeout,
                    };
                }
            }
            _ => {}
        }
    }
    if requested.is_some_and(|id| !indices.contains_key(&id)) {
        return Err(anyhow!("requested turn does not belong to this session"));
    }
    let selected_id = requested.or_else(|| turns.last().map(|turn| turn.turn_id));
    let selected = selected_id
        .map(|id| turn_details(projection, id))
        .transpose()?;
    Ok(AppSessionAnalysis {
        session_id,
        revision: projection
            .records
            .last()
            .map_or(0, |record| record.session_seq),
        turns,
        selected,
    })
}

fn turn_details(projection: &JournalProjection, turn_id: TurnId) -> Result<AppTurnAnalysis> {
    let mut detail = None;
    let mut steps: Vec<AppAnalysisStep> = Vec::new();
    let mut models = HashMap::new();
    let mut tools = HashMap::new();
    for record in projection
        .records
        .iter()
        .filter(|r| r.turn_id == Some(turn_id))
    {
        let new_step = |data| AppAnalysisStep {
            sequence: record.session_seq,
            execution_id: record.execution_id,
            thread_id: record.thread_id,
            started_at_ms: record.timestamp_ms,
            finished_at_ms: None,
            data,
        };
        match &record.entry {
            JournalEntry::TurnOpened(opened) => {
                detail = Some(AppTurnAnalysis {
                    turn_id,
                    task: opened.task.clone(),
                    module_epoch: opened.module_epoch,
                    config_snapshot: opened.config_snapshot.clone(),
                    steps: Vec::new(),
                    output: None,
                    error: None,
                });
            }
            JournalEntry::HookInvoked(trace) => {
                let mut step = new_step(AppAnalysisStepData::Hook {
                    trace: Box::new(trace.clone()),
                });
                step.finished_at_ms = Some(record.timestamp_ms);
                steps.push(step);
            }
            JournalEntry::ModelRequestRecorded(model) => {
                models.insert(model.exchange_id, steps.len());
                steps.push(new_step(AppAnalysisStepData::Model {
                    exchange_id: model.exchange_id,
                    origin: model.origin,
                    request: Box::new(model.request.clone()),
                    response: None,
                    failure: None,
                    messages: Vec::new(),
                }));
            }
            JournalEntry::ModelMessageRecorded(message) => {
                if let Some(&index) = models.get(&message.exchange_id)
                    && let AppAnalysisStepData::Model { messages, .. } = &mut steps[index].data
                {
                    messages.push(message.message.clone());
                }
            }
            JournalEntry::ModelResponseRecorded(model) => {
                if let Some(&index) = models.get(&model.exchange_id) {
                    let step = &mut steps[index];
                    step.finished_at_ms = Some(record.timestamp_ms);
                    if let AppAnalysisStepData::Model {
                        response, failure, ..
                    } = &mut step.data
                    {
                        match &model.outcome {
                            ModelResponseOutcome::Response { response: value } => {
                                *response = Some(Box::new(value.clone()))
                            }
                            ModelResponseOutcome::Error { failure: value } => {
                                *failure = Some(value.clone())
                            }
                        }
                    }
                }
            }
            JournalEntry::ToolCallRecorded(tool) => {
                let key = (record.execution_id, tool.call.id.clone());
                let index = *tools.entry(key).or_insert_with(|| {
                    steps.push(new_step(AppAnalysisStepData::Tool {
                        call: tool.call.clone(),
                        approval_reason: None,
                        resolution: None,
                        result: None,
                    }));
                    steps.len() - 1
                });
                if let AppAnalysisStepData::Tool {
                    approval_reason,
                    resolution,
                    ..
                } = &mut steps[index].data
                {
                    match &tool.phase {
                        ToolCallRecordPhase::Requested => {}
                        ToolCallRecordPhase::ApprovalRequested { reason } => {
                            *approval_reason = Some(reason.clone())
                        }
                        ToolCallRecordPhase::Resolved { resolution: value } => {
                            *resolution = Some(value.clone())
                        }
                    }
                }
            }
            JournalEntry::ToolEffectRecorded(tool) | JournalEntry::ToolResultRecorded(tool) => {
                if let Some(&index) = tools.get(&(record.execution_id, tool.result.call_id.clone()))
                {
                    steps[index].finished_at_ms = Some(record.timestamp_ms);
                    if let AppAnalysisStepData::Tool { result, .. } = &mut steps[index].data {
                        *result = Some(tool.result.clone());
                    }
                }
            }
            JournalEntry::HistoryMutated(history) => {
                if let Some(report) = &history.compaction {
                    let mut step = new_step(AppAnalysisStepData::Compaction {
                        report: report.clone(),
                        history_revision: history.new_revision,
                    });
                    step.finished_at_ms = Some(record.timestamp_ms);
                    steps.push(step);
                }
            }
            JournalEntry::TurnSettled(settled) => {
                if let Some(detail) = &mut detail {
                    detail.output = settled.output.clone();
                    detail.error = settled.error.clone();
                }
            }
        }
    }
    let mut detail = detail.ok_or_else(|| anyhow!("turn has no recorded admission"))?;
    detail.steps = steps;
    Ok(detail)
}

#[cfg(test)]
mod tests;
