use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    contracts::{CompactionInput, ToolExposureRequest},
    domain::{
        CacheHints, ContextBundle, Event, HistoryCompactionReport, ToolCall, ToolResult, ToolSpec,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, InstructionBlock,
        InstructionKind, TokenUsage,
    },
    plugin::{PluginWorkflowError, PluginWorkflowHostMut, PluginWorkflowInput},
};
use serde_json::json;

use super::{
    dynamic_tools,
    metadata::{insert_request_metadata_u32, insert_request_metadata_value, prompt_cache_key},
    token_accounting::{estimate_message_tokens, request_token_usage_snapshot},
};

pub(super) struct PreparedRequest {
    pub(super) request: CanonicalModelRequest,
    pub(super) compaction: Option<HistoryCompactionReport>,
}

struct CompactedMessages {
    messages: Vec<CanonicalMessage>,
    report: Option<HistoryCompactionReport>,
}

#[derive(Clone, Copy)]
struct RequestOptions {
    expose_tools: bool,
    include_dynamic_meta_tools: bool,
}

pub(super) fn request_from_state(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
    system_instructions: &str,
    developer_instructions: Option<&str>,
    phase: &str,
) -> Result<PreparedRequest, PluginWorkflowError> {
    request_from_state_with_instruction_blocks_and_options(
        input,
        host,
        messages,
        vec![InstructionBlock::new(
            InstructionKind::System,
            system_instructions,
            100,
        )],
        developer_instructions,
        phase,
        RequestOptions {
            expose_tools: true,
            include_dynamic_meta_tools: phase != "review",
        },
    )
}

pub(super) fn request_from_state_with_instruction_blocks(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
    instructions: Vec<InstructionBlock>,
    developer_instructions: Option<&str>,
    phase: &str,
) -> Result<PreparedRequest, PluginWorkflowError> {
    request_from_state_with_instruction_blocks_and_options(
        input,
        host,
        messages,
        instructions,
        developer_instructions,
        phase,
        RequestOptions {
            expose_tools: true,
            include_dynamic_meta_tools: phase != "review",
        },
    )
}

fn request_from_state_with_instruction_blocks_and_options(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
    mut instructions: Vec<InstructionBlock>,
    developer_instructions: Option<&str>,
    phase: &str,
    options: RequestOptions,
) -> Result<PreparedRequest, PluginWorkflowError> {
    let mut tools = if options.expose_tools {
        visible_tools(host, input)?
    } else {
        Vec::new()
    };
    let dynamic_tools_enabled = if options.expose_tools && options.include_dynamic_meta_tools {
        let all_visible_tools = dynamic_tools::all_policy_visible_tools(host, input)?;
        dynamic_tools::has_hidden_tools(&tools, &all_visible_tools)
    } else {
        false
    };
    if dynamic_tools_enabled {
        dynamic_tools::append_meta_tools(&mut tools, phase);
    }
    if let Some(developer_instructions) = developer_instructions {
        instructions.push(InstructionBlock::new(
            InstructionKind::Developer,
            developer_instructions,
            90,
        ));
    }
    if dynamic_tools_enabled {
        instructions.push(InstructionBlock::new(
            InstructionKind::Developer,
            dynamic_tools::INSTRUCTIONS,
            80,
        ));
    }
    let compacted = compact_messages(input, host, messages, phase)?;
    let mut request =
        CanonicalModelRequest::new(input.runtime.model_ref.clone(), compacted.messages)
            .with_instructions(instructions)
            .with_tools(tools)
            .with_reasoning(input.runtime.reasoning.clone())
            .with_cache(CacheHints::new(true, true));
    // Прокидываем потолок окна из capabilities в лимиты запроса, чтобы снимок
    // TokenUsageUpdated нёс max_input_tokens (хост-шейпер правит свою копию
    // уже после того, как плагин собрал снимок, поэтому делаем это здесь).
    request.limits.max_input_tokens = input.runtime.max_input_tokens;
    // Порог автокомпакта считает компактор (он владеет конфигом), а возвращает
    // его в отчёте. Кладём в metadata запроса, чтобы снимок взял именно его —
    // тогда метка на индикаторе контекста совпадает с реальным триггером.
    if let Some(trigger) = compacted
        .report
        .as_ref()
        .and_then(|report| report.trigger_tokens)
    {
        insert_request_metadata_u32(&mut request, "compaction_trigger_tokens", trigger);
    }
    let prompt_cache_key = prompt_cache_key(input, &request);
    insert_request_metadata_value(&mut request, "prompt_cache_key", json!(prompt_cache_key));
    Ok(PreparedRequest {
        request,
        compaction: compacted.report,
    })
}

pub(super) fn execute_or_handle_tool(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
    phase: &str,
) -> Result<ToolResult, PluginWorkflowError> {
    if dynamic_tools::is_meta_tool(&call.name) {
        dynamic_tools::handle_meta_tool_call(host, input, call, phase)
    } else {
        execute_tool(host, input, call)
    }
}

fn compact_messages(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
    reason: &str,
) -> Result<CompactedMessages, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let compaction_input = CompactionInput::new(
        input.task.clone(),
        input.runtime.model_ref.clone(),
        messages.to_vec(),
    )
    .with_reason(reason)
    .with_token_estimate(estimate_message_tokens(messages))
    // window_tokens — сырое окно, из него компактор берёт trigger_fraction;
    // max_tokens оставляем как legacy-fallback на случай отсутствия конфига.
    .with_window_tokens(input.runtime.max_input_tokens)
    .with_max_tokens(model_auto_compact_limit(input.runtime.max_input_tokens));
    let input_json = to_json_string(&compaction_input)?;
    let output_json = match host.compact_history_json(RString::from(input_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    let output: proteus_contracts::contracts::CompactionOutput =
        from_json_string(output_json.as_str())?;
    if output.messages.is_empty() && !messages.is_empty() {
        return Err(PluginWorkflowError::new(
            "compactor returned empty messages for non-empty history",
        ));
    }
    let report = HistoryCompactionReport::from_compaction_output(&compaction_input, &output);
    Ok(CompactedMessages {
        messages: output.messages,
        report: Some(report),
    })
}

pub(super) fn build_context(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
) -> Result<ContextBundle, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let task_json = to_json_string(&input.task)?;
    let bundle_json = match host.build_context_json(RString::from(task_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    from_json_string(bundle_json.as_str())
}

pub(super) fn complete_model(
    host: &mut PluginWorkflowHostMut<'_>,
    request: &CanonicalModelRequest,
    phase: &str,
) -> Result<CanonicalModelResponse, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let request_json = to_json_string(request)?;
    let response_json = match host.complete_model_json(RString::from(request_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    let response: CanonicalModelResponse = from_json_string(response_json.as_str())?;
    emit_token_usage(host, request, response.usage.clone(), phase)?;
    Ok(response)
}

fn visible_tools(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
) -> Result<Vec<ToolSpec>, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let request = ToolExposureRequest::new(input.task.clone()).with_reason("before_model_request");
    let request_json = to_json_string(&request)?;
    let tools_json = match host.select_tools_json(RString::from(request_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    let output: proteus_contracts::contracts::ToolExposureOutput =
        from_json_string(tools_json.as_str())?;
    Ok(output.tools)
}

pub(super) fn execute_tool(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
) -> Result<ToolResult, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let task_json = to_json_string(&input.task)?;
    let call_json = to_json_string(call)?;
    let result_json = match host
        .execute_tool_json(RString::from(task_json), RString::from(call_json))
    {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    from_json_string(result_json.as_str())
}

fn ensure_not_cancelled(host: &mut PluginWorkflowHostMut<'_>) -> Result<(), PluginWorkflowError> {
    match host.is_cancelled() {
        RResult::ROk(false) => Ok(()),
        RResult::ROk(true) => Err(PluginWorkflowError::new("turn canceled by client")),
        RResult::RErr(error) => Err(PluginWorkflowError::new(error.message.into_string())),
    }
}

pub(super) fn emit_event(
    host: &mut PluginWorkflowHostMut<'_>,
    event: &Event,
) -> Result<(), PluginWorkflowError> {
    let event_json = to_json_string(event)?;
    match host.emit_event_json(RString::from(event_json)) {
        RResult::ROk(()) => Ok(()),
        RResult::RErr(error) => Err(PluginWorkflowError::new(error.message.into_string())),
    }
}

pub(super) fn to_json_string<T: serde::Serialize>(
    value: &T,
) -> Result<String, PluginWorkflowError> {
    serde_json::to_string(value).map_err(|error| PluginWorkflowError::new(error.to_string()))
}

pub(super) fn from_json_string<T: serde::de::DeserializeOwned>(
    value: &str,
) -> Result<T, PluginWorkflowError> {
    serde_json::from_str(value).map_err(|error| PluginWorkflowError::new(error.to_string()))
}

fn model_auto_compact_limit(max_input_tokens: Option<u32>) -> Option<u32> {
    max_input_tokens.map(|tokens| {
        let limit = (u64::from(tokens) * 8 / 10).max(1);
        u32::try_from(limit).unwrap_or(u32::MAX)
    })
}

fn emit_token_usage(
    host: &mut PluginWorkflowHostMut<'_>,
    request: &CanonicalModelRequest,
    actual: Option<TokenUsage>,
    phase: &str,
) -> Result<(), PluginWorkflowError> {
    let usage = request_token_usage_snapshot(request, actual, phase);
    emit_event(host, &Event::TokenUsageUpdated { usage })
}
