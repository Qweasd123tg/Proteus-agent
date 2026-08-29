use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    contracts::Model,
    core::{JournalEntry, ModelResponseOutcome, SessionStore, normalize_session_dir_path},
    domain::{ExchangeId, ExecutionId, ModelRef, SessionId, ThreadId, ToolSurface, TurnId},
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ContentPart, ModelStreamEvent, TokenUsage,
        validate_model_response_against_request,
    },
};

pub const PROMPT_REPLAY_REPORT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PromptReplayOptions {
    pub exchange_id: Option<ExchangeId>,
    pub allow_hosted_tools: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PromptReplayReport {
    pub schema_version: u32,
    pub source: PromptReplaySource,
    pub recorded_model: ModelRef,
    pub replay_model: ModelRef,
    pub replay_adapter: String,
    pub recorded_outcome: PromptReplayOutcomeSummary,
    pub replay_outcome: PromptReplayOutcomeSummary,
    pub usage: PromptReplayUsage,
    pub text_equal: Option<bool>,
    pub local_tool_calls: PromptReplayCounts,
    pub local_tool_call_names: PromptReplayNames,
    pub hosted_activities: PromptReplayCounts,
    pub citations: PromptReplayCounts,
    pub request_hosted_tools: Vec<String>,
    pub hosted_tools_allowed: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptReplaySource {
    pub journal_path: PathBuf,
    pub session_id: SessionId,
    pub execution_id: ExecutionId,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub exchange_id: ExchangeId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptReplayOutcomeSummary {
    pub status: PromptReplayOutcomeStatus,
    pub finish_reason: Option<String>,
    pub error: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptReplayOutcomeStatus {
    Response,
    Error,
}

impl PromptReplayOutcomeStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Response => "response",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptReplayUsage {
    pub recorded: Option<TokenUsage>,
    pub replay: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptReplayCounts {
    pub recorded: usize,
    pub replay: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptReplayNames {
    pub recorded: Vec<String>,
    pub replay: Vec<String>,
}

/// Replays one recorded post-shaping request through a raw model adapter.
///
/// The caller must pass the adapter itself rather than `ModelService`: this
/// boundary deliberately skips request shaping, journal recording and runtime
/// delta emission. The returned response is reported only; local tool calls
/// are never executed here.
pub async fn replay_prompt(
    path: impl AsRef<Path>,
    model: Arc<dyn Model>,
    options: PromptReplayOptions,
) -> Result<PromptReplayReport> {
    let session_dir = normalize_session_dir_path(path.as_ref().to_path_buf())?;
    let store = SessionStore::open(session_dir.clone()).with_context(|| {
        format!(
            "failed to open canonical session journal at {}",
            session_dir.display()
        )
    })?;
    let projection = store.load_projection()?;
    let exchange = select_exchange(&projection.records, options.exchange_id)?;
    let request_hosted_tools = hosted_tool_names(&exchange.request);
    if !request_hosted_tools.is_empty() && !options.allow_hosted_tools {
        bail!(
            "model exchange {} includes provider-hosted tools [{}]; prompt replay is disabled by default because the provider may perform external side effects; rerun with --allow-hosted-tools to send the saved canonical request unchanged",
            exchange.exchange_id,
            request_hosted_tools.join(", ")
        );
    }

    let replay_adapter = model.id().into_owned();
    let replay_model = exchange.request.model.clone();
    let started = Instant::now();
    let replay_outcome = match invoke_model(model, exchange.request.clone()).await {
        Ok(response) => ModelResponseOutcome::Response { response },
        Err(error) => ModelResponseOutcome::Error {
            message: format!("{error:#}"),
        },
    };
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(build_report(
        store.journal_path(),
        store.session_id(),
        exchange,
        replay_model,
        replay_adapter,
        replay_outcome,
        request_hosted_tools,
        options.allow_hosted_tools,
        duration_ms,
    ))
}

struct RecordedExchange {
    exchange_id: ExchangeId,
    execution_id: ExecutionId,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
    request: CanonicalModelRequest,
    outcome: ModelResponseOutcome,
}

struct PendingExchange {
    exchange_id: ExchangeId,
    execution_id: ExecutionId,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
    request: CanonicalModelRequest,
}

fn select_exchange(
    records: &[crate::core::JournalRecord],
    requested_id: Option<ExchangeId>,
) -> Result<RecordedExchange> {
    let mut requests = Vec::new();
    let mut outcomes = HashMap::new();
    for record in records {
        match &record.entry {
            JournalEntry::ModelRequestRecorded(recorded) => {
                let execution_id = record.execution_id.ok_or_else(|| {
                    anyhow!(
                        "model exchange {} request is missing execution_id",
                        recorded.exchange_id
                    )
                })?;
                requests.push(PendingExchange {
                    exchange_id: recorded.exchange_id,
                    execution_id,
                    thread_id: record.thread_id,
                    turn_id: record.turn_id,
                    request: recorded.request.clone(),
                });
            }
            JournalEntry::ModelResponseRecorded(recorded) => {
                outcomes.insert(recorded.exchange_id, recorded.outcome.clone());
            }
            _ => {}
        }
    }

    if requests.is_empty() {
        bail!("canonical session journal contains no model exchanges");
    }
    let available = available_exchange_ids(&requests, &outcomes);
    let selected = match requested_id {
        Some(exchange_id) => requests
            .into_iter()
            .find(|exchange| exchange.exchange_id == exchange_id)
            .ok_or_else(|| {
                anyhow!(
                    "model exchange {exchange_id} was not found; available exchange IDs: {available}"
                )
            })?,
        None if requests.len() == 1 => requests
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("canonical session journal contains no model exchanges"))?,
        None => bail!(
            "canonical session journal contains multiple model exchanges; pass --exchange-id <id>; available exchange IDs: {available}"
        ),
    };
    let outcome = outcomes.remove(&selected.exchange_id).ok_or_else(|| {
        anyhow!(
            "model exchange {} is incomplete: model_request_recorded has no model_response_recorded",
            selected.exchange_id
        )
    })?;

    Ok(RecordedExchange {
        exchange_id: selected.exchange_id,
        execution_id: selected.execution_id,
        thread_id: selected.thread_id,
        turn_id: selected.turn_id,
        request: selected.request,
        outcome,
    })
}

fn available_exchange_ids(
    requests: &[PendingExchange],
    outcomes: &HashMap<ExchangeId, ModelResponseOutcome>,
) -> String {
    requests
        .iter()
        .map(|exchange| {
            if outcomes.contains_key(&exchange.exchange_id) {
                exchange.exchange_id.to_string()
            } else {
                format!("{} (incomplete)", exchange.exchange_id)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

async fn invoke_model(
    model: Arc<dyn Model>,
    request: CanonicalModelRequest,
) -> Result<CanonicalModelResponse> {
    let validation_request = request.clone();
    let mut stream = model.stream(request).await?;
    while let Some(event) = stream.next().await {
        match event? {
            ModelStreamEvent::Response { response } => {
                validate_model_response_against_request(&validation_request, &response)
                    .map_err(|error| anyhow!("invalid replay model response: {error}"))?;
                return Ok(response);
            }
            ModelStreamEvent::Error { message } => {
                bail!("model stream error: {message}");
            }
            _ => {}
        }
    }
    bail!("model stream ended without a complete response")
}

#[allow(clippy::too_many_arguments)]
fn build_report(
    journal_path: PathBuf,
    session_id: SessionId,
    exchange: RecordedExchange,
    replay_model: ModelRef,
    replay_adapter: String,
    replay_outcome: ModelResponseOutcome,
    request_hosted_tools: Vec<String>,
    hosted_tools_allowed: bool,
    duration_ms: u64,
) -> PromptReplayReport {
    let recorded_response = response_from_outcome(&exchange.outcome);
    let replay_response = response_from_outcome(&replay_outcome);
    let recorded_text = text_from_outcome(&exchange.outcome);
    let replay_text = text_from_outcome(&replay_outcome);
    let text_equal = recorded_text
        .as_ref()
        .zip(replay_text.as_ref())
        .map(|(recorded, replay)| recorded == replay);

    PromptReplayReport {
        schema_version: PROMPT_REPLAY_REPORT_SCHEMA_VERSION,
        source: PromptReplaySource {
            journal_path,
            session_id,
            execution_id: exchange.execution_id,
            thread_id: exchange.thread_id,
            turn_id: exchange.turn_id,
            exchange_id: exchange.exchange_id,
        },
        recorded_model: exchange.request.model,
        replay_model,
        replay_adapter,
        recorded_outcome: outcome_summary(&exchange.outcome, recorded_text),
        replay_outcome: outcome_summary(&replay_outcome, replay_text),
        usage: PromptReplayUsage {
            recorded: recorded_response.and_then(|response| response.usage.clone()),
            replay: replay_response.and_then(|response| response.usage.clone()),
        },
        text_equal,
        local_tool_calls: PromptReplayCounts {
            recorded: recorded_response.map_or(0, |response| response.tool_calls.len()),
            replay: replay_response.map_or(0, |response| response.tool_calls.len()),
        },
        local_tool_call_names: PromptReplayNames {
            recorded: local_tool_call_names(recorded_response),
            replay: local_tool_call_names(replay_response),
        },
        hosted_activities: PromptReplayCounts {
            recorded: count_parts(recorded_response, is_hosted_activity),
            replay: count_parts(replay_response, is_hosted_activity),
        },
        citations: PromptReplayCounts {
            recorded: count_parts(recorded_response, is_citation),
            replay: count_parts(replay_response, is_citation),
        },
        request_hosted_tools,
        hosted_tools_allowed,
        duration_ms,
    }
}

fn response_from_outcome(outcome: &ModelResponseOutcome) -> Option<&CanonicalModelResponse> {
    match outcome {
        ModelResponseOutcome::Response { response } => Some(response),
        ModelResponseOutcome::Error { .. } => None,
    }
}

fn outcome_summary(
    outcome: &ModelResponseOutcome,
    text: Option<String>,
) -> PromptReplayOutcomeSummary {
    match outcome {
        ModelResponseOutcome::Response { response } => PromptReplayOutcomeSummary {
            status: PromptReplayOutcomeStatus::Response,
            finish_reason: Some(finish_reason_name(&response.finish_reason).to_owned()),
            error: None,
            text,
        },
        ModelResponseOutcome::Error { message } => PromptReplayOutcomeSummary {
            status: PromptReplayOutcomeStatus::Error,
            finish_reason: None,
            error: Some(message.clone()),
            text: None,
        },
    }
}

fn finish_reason_name(reason: &crate::model_standard::FinishReason) -> &'static str {
    use crate::model_standard::FinishReason;

    match reason {
        FinishReason::Stop => "stop",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::Length => "length",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Error => "error",
        FinishReason::Unknown => "unknown",
        _ => "unknown",
    }
}

fn text_from_outcome(outcome: &ModelResponseOutcome) -> Option<String> {
    response_from_outcome(outcome).map(|response| {
        response
            .message
            .parts
            .iter()
            .filter_map(|part| match &part.payload {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>()
    })
}

fn local_tool_call_names(response: Option<&CanonicalModelResponse>) -> Vec<String> {
    response
        .into_iter()
        .flat_map(|response| response.tool_calls.iter())
        .map(|call| call.name.clone())
        .collect()
}

fn count_parts(
    response: Option<&CanonicalModelResponse>,
    predicate: fn(&ContentPart) -> bool,
) -> usize {
    response.map_or(0, |response| {
        response
            .message
            .parts
            .iter()
            .filter(|part| predicate(&part.payload))
            .count()
    })
}

fn is_hosted_activity(part: &ContentPart) -> bool {
    matches!(part, ContentPart::HostedToolActivity { .. })
}

fn is_citation(part: &ContentPart) -> bool {
    matches!(part, ContentPart::Citation { .. })
}

fn hosted_tool_names(request: &CanonicalModelRequest) -> Vec<String> {
    request
        .tools
        .iter()
        .filter(|tool| matches!(tool.surface, ToolSurface::ProviderHosted { .. }))
        .map(|tool| tool.name.clone())
        .collect()
}

#[cfg(test)]
mod tests;
