use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, MutexGuard},
};

use anyhow::{Result, anyhow, bail};

use crate::{
    contracts::{ApprovalResponse, CompactionInput, CompactionOutput, ToolExposureOutput},
    core::ModelResponseOutcome,
    domain::{
        CacheHints, CallId, ContextBundle, HistoryCompactionReport, HostedToolKind,
        ReasoningConfig, ToolCall, ToolCallResolution, ToolResult, ToolSurface,
    },
    model_standard::{CanonicalModelRequest, ModelCapabilities},
};

use super::{
    fixture::{RecordedModelExchange, RecordedToolInvocation},
    normalize::{
        calls_equal, messages_equal, outputs_equal, request_difference, requests_equal,
        results_equal, rewrite_result_call_ids,
    },
};

mod adapters;

pub(super) use adapters::{
    ReplayApprovalTransport, ReplayCompactor, ReplayContextBuilder, ReplayModel,
    ReplayToolExposure, register_replay_tools,
};

pub(super) struct ReplayState {
    inner: Mutex<ReplayStateInner>,
    capabilities: ModelCapabilities,
    context: ContextBundle,
    registered_tool_names: HashSet<String>,
}

struct ReplayStateInner {
    exchanges: Vec<RecordedModelExchange>,
    next_exchange: usize,
    tools: Vec<ExpectedToolState>,
    actual_to_expected: HashMap<CallId, CallId>,
    expected_to_actual: HashMap<CallId, CallId>,
    compactions: Vec<ExpectedCompaction>,
    issues: Vec<String>,
}

struct ExpectedToolState {
    recorded: RecordedToolInvocation,
    requested: bool,
    approval_requested: bool,
    resolved: bool,
    result_recorded: bool,
}

struct ExpectedCompaction {
    report: HistoryCompactionReport,
    consumed: bool,
}

pub(super) struct ReplaySummary {
    pub model_exchanges: usize,
    pub tool_calls: usize,
    pub issues: Vec<String>,
}

impl ReplayState {
    pub fn new(
        exchanges: Vec<RecordedModelExchange>,
        tools: Vec<RecordedToolInvocation>,
        compactions: Vec<HistoryCompactionReport>,
        context: ContextBundle,
        registered_tool_names: HashSet<String>,
        snapshot_reasoning: &ReasoningConfig,
    ) -> Self {
        let capabilities = replay_capabilities(&exchanges[0].request, snapshot_reasoning);
        Self {
            inner: Mutex::new(ReplayStateInner {
                exchanges,
                next_exchange: 0,
                tools: tools
                    .into_iter()
                    .map(|recorded| ExpectedToolState {
                        recorded,
                        requested: false,
                        approval_requested: false,
                        resolved: false,
                        result_recorded: false,
                    })
                    .collect(),
                actual_to_expected: HashMap::new(),
                expected_to_actual: HashMap::new(),
                compactions: compactions
                    .into_iter()
                    .map(|report| ExpectedCompaction {
                        report,
                        consumed: false,
                    })
                    .collect(),
                issues: Vec::new(),
            }),
            capabilities,
            context,
            registered_tool_names,
        }
    }

    pub fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }

    pub fn context(&self) -> ContextBundle {
        self.context.clone()
    }

    pub fn current_request(&self) -> Result<CanonicalModelRequest> {
        let inner = self.lock();
        inner
            .exchanges
            .get(inner.next_exchange)
            .map(|exchange| exchange.request.clone())
            .ok_or_else(|| anyhow!("workflow requested another host capability after all recorded model exchanges were consumed"))
    }

    pub fn consume_model_request(
        &self,
        actual: &CanonicalModelRequest,
    ) -> Result<ModelResponseOutcome> {
        let mut inner = self.lock();
        let index = inner.next_exchange;
        let Some(expected) = inner.exchanges.get(index).cloned() else {
            return mismatch(
                &mut inner,
                format!(
                    "workflow emitted unexpected model request #{} after the recorded exchange script ended",
                    index + 1
                ),
            );
        };
        if !requests_equal(actual, &expected.request, &inner.actual_to_expected) {
            let difference =
                request_difference(actual, &expected.request, &inner.actual_to_expected);
            return mismatch(
                &mut inner,
                format!(
                    "model request #{} does not match recorded exchange {} ({difference})",
                    index + 1,
                    expected.exchange_id
                ),
            );
        }
        let outcome = expected.outcome.clone();
        inner.next_exchange += 1;
        Ok(outcome)
    }

    pub fn tool_exposure(&self) -> Result<ToolExposureOutput> {
        let request = self.current_request()?;
        let tools = request
            .tools
            .into_iter()
            .filter(|tool| self.registered_tool_names.contains(&tool.name))
            .collect();
        let metadata = request
            .metadata
            .get("tool_exposure")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let mut output = ToolExposureOutput::new(tools);
        output.metadata = metadata;
        Ok(output)
    }

    pub fn compact(&self, input: CompactionInput) -> Result<CompactionOutput> {
        let expected = self.current_request()?;
        let mut inner = self.lock();
        let equal = messages_equal(
            &input.messages,
            &expected.messages,
            &inner.actual_to_expected,
        );
        let mut metadata = serde_json::Map::new();
        if let Some(trigger) = expected
            .metadata
            .get("compaction_trigger_tokens")
            .and_then(serde_json::Value::as_u64)
        {
            metadata.insert("trigger_tokens".to_owned(), trigger.into());
        }
        if equal {
            let mut output = CompactionOutput::unchanged(input.messages);
            output.token_estimate = input.token_estimate;
            output.metadata = serde_json::Value::Object(metadata);
            return Ok(output);
        }

        let reason = input.reason.as_deref();
        let report_index = inner
            .compactions
            .iter()
            .position(|candidate| {
                !candidate.consumed && candidate.report.reason.as_deref() == reason
            })
            .or_else(|| {
                inner
                    .compactions
                    .iter()
                    .position(|candidate| !candidate.consumed)
            });
        let Some(report_index) = report_index else {
            return mismatch(
                &mut inner,
                format!(
                    "compactor input for phase {} differs from the recorded model request, but the journal contains no matching changed compaction",
                    reason.unwrap_or("unknown")
                ),
            );
        };
        let report = &mut inner.compactions[report_index];
        report.consumed = true;
        let recorded = report.report.clone();
        insert_compaction_report_metadata(&mut metadata, &recorded);
        let mut output = CompactionOutput::changed(expected.messages, recorded.summary);
        output.token_estimate = recorded.output_token_estimate;
        output.metadata = serde_json::Value::Object(metadata);
        Ok(output)
    }

    pub fn approval_response(&self, actual_call_id: &str) -> Result<ApprovalResponse> {
        let inner = self.lock();
        let expected = expected_tool(&inner, actual_call_id)?;
        match &expected.recorded.resolution {
            ToolCallResolution::Approved => Ok(ApprovalResponse::approve()),
            ToolCallResolution::ApprovalDenied { reason } => {
                Ok(ApprovalResponse::deny(reason.clone()))
            }
            resolution => bail!(
                "workflow requested replay approval for tool call {}, but recorded resolution is {resolution:?}",
                expected.recorded.call.id
            ),
        }
    }

    pub fn replay_tool_result(&self, actual_call_id: &str) -> Result<ToolResult> {
        let inner = self.lock();
        let expected = expected_tool(&inner, actual_call_id)?;
        if !expected.recorded.resolution.permits_side_effect() {
            bail!(
                "workflow tried to invoke replay tool {}, but recorded resolution {:?} did not permit invocation",
                expected.recorded.call.id,
                expected.recorded.resolution
            );
        }
        let mut result = expected.recorded.result.clone();
        rewrite_result_call_ids(&mut result, &inner.expected_to_actual);
        result.call_id = actual_call_id.to_owned();
        Ok(result)
    }

    pub fn summary(&self) -> ReplaySummary {
        let mut inner = self.lock();
        let model_exchanges = inner.next_exchange;
        let tool_calls = inner.tools.iter().filter(|tool| tool.requested).count();
        if model_exchanges != inner.exchanges.len() {
            let total = inner.exchanges.len();
            inner.issues.push(format!(
                "workflow consumed {model_exchanges} of {total} recorded model exchanges"
            ));
        }
        let missing = inner
            .tools
            .iter()
            .filter(|tool| {
                !tool.requested
                    || !tool.resolved
                    || !tool.result_recorded
                    || (tool.recorded.approval_reason.is_some() && !tool.approval_requested)
            })
            .map(|tool| tool.recorded.call.id.clone())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            inner.issues.push(format!(
                "workflow did not consume complete lifecycle for recorded tool calls: {}",
                missing.join(", ")
            ));
        }
        let unconsumed_compactions = inner
            .compactions
            .iter()
            .filter(|compaction| !compaction.consumed)
            .count();
        if unconsumed_compactions > 0 {
            inner.issues.push(format!(
                "workflow did not reproduce {unconsumed_compactions} recorded changed compaction(s)"
            ));
        }
        ReplaySummary {
            model_exchanges,
            tool_calls,
            issues: inner.issues.clone(),
        }
    }

    pub fn histories_equal(
        &self,
        actual: &[crate::model_standard::CanonicalMessage],
        expected: &[crate::model_standard::CanonicalMessage],
    ) -> bool {
        let inner = self.lock();
        messages_equal(actual, expected, &inner.actual_to_expected)
    }

    pub fn outputs_equal(
        &self,
        actual: &crate::domain::AgentOutput,
        expected: &crate::domain::AgentOutput,
    ) -> bool {
        let inner = self.lock();
        outputs_equal(actual, expected, &inner.actual_to_expected)
    }

    fn record_tool_requested(&self, call: &ToolCall) -> Result<()> {
        let mut inner = self.lock();
        let index = match_tool_index(&inner, call)?;
        if inner.tools[index].requested {
            return mismatch(
                &mut inner,
                format!(
                    "tool call {} was requested more than once during replay",
                    call.id
                ),
            );
        }
        let expected_id = inner.tools[index].recorded.call.id.clone();
        inner
            .actual_to_expected
            .insert(call.id.clone(), expected_id.clone());
        inner
            .expected_to_actual
            .insert(expected_id.clone(), call.id.clone());
        if !calls_equal(
            call,
            &inner.tools[index].recorded.call,
            &inner.actual_to_expected,
        ) {
            return mismatch(
                &mut inner,
                format!(
                    "tool call {} does not match recorded call {expected_id}",
                    call.id
                ),
            );
        }
        inner.tools[index].requested = true;
        Ok(())
    }

    fn record_approval_requested(&self, call: &ToolCall, reason: &str) -> Result<()> {
        let mut inner = self.lock();
        let index = mapped_tool_index(&inner, &call.id)?;
        let expected_reason = inner.tools[index].recorded.approval_reason.clone();
        let expected_call_id = inner.tools[index].recorded.call.id.clone();
        if expected_reason.as_deref() != Some(reason) {
            return mismatch(
                &mut inner,
                format!(
                    "approval reason for tool call {} differs: recorded={expected_reason:?}, replay={reason:?}",
                    expected_call_id
                ),
            );
        }
        inner.tools[index].approval_requested = true;
        Ok(())
    }

    fn record_tool_resolved(&self, call: &ToolCall, resolution: &ToolCallResolution) -> Result<()> {
        let mut inner = self.lock();
        let index = mapped_tool_index(&inner, &call.id)?;
        let expected_call_id = inner.tools[index].recorded.call.id.clone();
        let expected_resolution = inner.tools[index].recorded.resolution.clone();
        if expected_resolution != *resolution {
            return mismatch(
                &mut inner,
                format!(
                    "tool resolution for {} differs: recorded={:?}, replay={resolution:?}",
                    expected_call_id, expected_resolution
                ),
            );
        }
        inner.tools[index].resolved = true;
        Ok(())
    }

    fn record_tool_result(&self, result: &ToolResult) -> Result<()> {
        let mut inner = self.lock();
        let index = mapped_tool_index(&inner, &result.call_id)?;
        let expected_call_id = inner.tools[index].recorded.call.id.clone();
        let expected_result = inner.tools[index].recorded.result.clone();
        if !results_equal(result, &expected_result, &inner.actual_to_expected) {
            return mismatch(
                &mut inner,
                format!(
                    "tool result for {} differs from the recorded result",
                    expected_call_id
                ),
            );
        }
        inner.tools[index].result_recorded = true;
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, ReplayStateInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn replay_capabilities(
    request: &CanonicalModelRequest,
    snapshot_reasoning: &ReasoningConfig,
) -> ModelCapabilities {
    let hosted = request
        .tools
        .iter()
        .filter_map(|tool| match &tool.surface {
            ToolSurface::ProviderHosted { config } => Some(config.kind()),
            _ => None,
        })
        .collect::<Vec<HostedToolKind>>();
    ModelCapabilities::empty()
        .with_tools(true)
        .with_parallel_tool_calls(true)
        .with_freeform_tools(
            request
                .tools
                .iter()
                .any(|tool| matches!(tool.surface, ToolSurface::Freeform { .. })),
        )
        .with_streaming(true)
        .with_json_schema(true)
        .with_system_role(true)
        .with_developer_role(true)
        .with_cache_hints(request.cache != CacheHints::default())
        .with_reasoning_config(
            request.reasoning != ReasoningConfig::default()
                || *snapshot_reasoning == ReasoningConfig::default(),
        )
        .with_provider_hosted_tools(hosted)
        .with_max_input_tokens(request.limits.max_input_tokens)
        .with_max_output_tokens(request.limits.max_output_tokens)
}

fn insert_compaction_report_metadata(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
    report: &HistoryCompactionReport,
) {
    metadata.insert("input_messages".to_owned(), report.input_messages.into());
    metadata.insert("output_messages".to_owned(), report.output_messages.into());
    for (key, value) in [
        ("original_token_estimate", report.original_token_estimate),
        ("output_token_estimate", report.output_token_estimate),
        ("trigger_tokens", report.trigger_tokens),
    ] {
        if let Some(value) = value {
            metadata.insert(key.to_owned(), value.into());
        }
    }
    for (key, value) in [
        ("summary_source", report.summary_source.as_ref()),
        ("skipped_reason", report.skipped_reason.as_ref()),
    ] {
        if let Some(value) = value {
            metadata.insert(key.to_owned(), value.clone().into());
        }
    }
    if let Some(recorded) = report.metadata.as_object() {
        metadata.extend(recorded.clone());
    }
}

fn match_tool_index(inner: &ReplayStateInner, actual: &ToolCall) -> Result<usize> {
    if let Some(index) = inner
        .tools
        .iter()
        .position(|tool| !tool.requested && tool.recorded.call.id == actual.id)
    {
        return Ok(index);
    }
    inner
        .tools
        .iter()
        .position(|tool| {
            !tool.requested
                && tool.recorded.call.name == actual.name
                && tool.recorded.call.args == actual.args
                && tool.recorded.call.surface == actual.surface
                && tool.recorded.call.raw_arguments == actual.raw_arguments
        })
        .ok_or_else(|| {
            anyhow!(
                "unexpected replay tool call {} ({})",
                actual.id,
                actual.name
            )
        })
}

fn mapped_tool_index(inner: &ReplayStateInner, actual_call_id: &str) -> Result<usize> {
    let expected_id = inner
        .actual_to_expected
        .get(actual_call_id)
        .ok_or_else(|| anyhow!("replay tool call {actual_call_id} was not requested"))?;
    inner
        .tools
        .iter()
        .position(|tool| tool.recorded.call.id == *expected_id)
        .ok_or_else(|| anyhow!("recorded tool call {expected_id} is missing from replay state"))
}

fn expected_tool<'a>(
    inner: &'a ReplayStateInner,
    actual_call_id: &str,
) -> Result<&'a ExpectedToolState> {
    let index = mapped_tool_index(inner, actual_call_id)?;
    inner
        .tools
        .get(index)
        .ok_or_else(|| anyhow!("replay tool index {index} is invalid"))
}

fn mismatch<T>(inner: &mut ReplayStateInner, message: String) -> Result<T> {
    inner.issues.push(message.clone());
    bail!(message)
}
