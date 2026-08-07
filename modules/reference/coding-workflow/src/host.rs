use proteus_contracts::{
    contracts::{CompactionInput, ToolExposureRequest},
    domain::{
        CacheHints, ContextBundle, Event, HistoryCompactionReport, ToolCall, ToolResult, ToolSpec,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, InstructionBlock,
        InstructionKind, TokenUsage,
    },
    process_module::{ProcessModuleError, WorkflowModuleHostMut, WorkflowModuleInput},
};

use super::{
    dynamic_tools,
    metadata::{cache_routing_key, insert_request_metadata_u32, insert_request_metadata_value},
    token_accounting::{LastModelUsage, effective_token_estimate, request_token_usage_snapshot},
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
struct RequestOptions<'a> {
    expose_tools: bool,
    include_dynamic_meta_tools: bool,
    last_usage: Option<&'a LastModelUsage>,
}

pub(super) fn request_from_state(
    input: &WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
    messages: &[CanonicalMessage],
    system_instructions: &str,
    developer_instructions: Option<&str>,
    phase: &str,
    last_usage: Option<&LastModelUsage>,
) -> Result<PreparedRequest, ProcessModuleError> {
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
            last_usage,
        },
    )
}

pub(super) fn request_from_state_with_instruction_blocks(
    input: &WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
    messages: &[CanonicalMessage],
    instructions: Vec<InstructionBlock>,
    developer_instructions: Option<&str>,
    phase: &str,
    last_usage: Option<&LastModelUsage>,
) -> Result<PreparedRequest, ProcessModuleError> {
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
            last_usage,
        },
    )
}

fn request_from_state_with_instruction_blocks_and_options(
    input: &WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
    messages: &[CanonicalMessage],
    mut instructions: Vec<InstructionBlock>,
    developer_instructions: Option<&str>,
    phase: &str,
    options: RequestOptions<'_>,
) -> Result<PreparedRequest, ProcessModuleError> {
    let selected = if options.expose_tools {
        visible_tools(host, input, phase)?
    } else {
        SelectedTools::empty()
    };
    let exposure_metadata = selected.metadata;
    let mut tools = selected.tools;
    let dynamic_tools_enabled = if options.expose_tools && options.include_dynamic_meta_tools {
        let all_candidate_tools = dynamic_tools::all_policy_visible_tools(host, input)?;
        dynamic_tools::has_hidden_tools(&tools, &all_candidate_tools)
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
    let compacted = compact_messages(input, host, messages, phase, options.last_usage)?;
    let mut request =
        CanonicalModelRequest::new(input.runtime.model_ref.clone(), compacted.messages)
            .with_instructions(instructions)
            .with_tools(tools)
            .with_reasoning(input.runtime.reasoning.clone())
            .with_cache(CacheHints::new(true, true).with_routing_key(cache_routing_key(input)));
    // Прокидываем потолок окна из capabilities в лимиты запроса, чтобы снимок
    // TokenUsageUpdated нёс max_input_tokens (хост-шейпер правит свою копию
    // уже после того, как module собрал снимок, поэтому делаем это здесь).
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
    // Telemetry селектора (hidden count, saved schema tokens и т.п.) не должна
    // теряться на workflow-границе: кладём её в metadata запроса, откуда её
    // видят снимки usage и event log.
    if !exposure_metadata.is_null() {
        insert_request_metadata_value(&mut request, "tool_exposure", exposure_metadata);
    }
    Ok(PreparedRequest {
        request,
        compaction: compacted.report,
    })
}

pub(super) fn execute_or_handle_tool(
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
    call: &ToolCall,
    phase: &str,
) -> Result<ToolResult, ProcessModuleError> {
    if dynamic_tools::is_meta_tool(&call.name) {
        dynamic_tools::handle_meta_tool_call(host, input, call, phase)
    } else {
        execute_tool(host, input, call)
    }
}

fn compact_messages(
    input: &WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
    messages: &[CanonicalMessage],
    reason: &str,
    last_usage: Option<&LastModelUsage>,
) -> Result<CompactedMessages, ProcessModuleError> {
    ensure_not_cancelled(host)?;
    let compaction_input = CompactionInput::new(
        input.task.clone(),
        input.runtime.model_ref.clone(),
        messages.to_vec(),
    )
    .with_reason(reason)
    .with_token_estimate(effective_token_estimate(messages, last_usage))
    .with_window_tokens(input.runtime.max_input_tokens);
    let input_json = to_json_string(&compaction_input)?;
    let output_json = match host.compact_history_json(String::from(input_json)) {
        Ok(json) => json,
        Err(error) => return Err(ProcessModuleError::new(error.message)),
    };
    let output: proteus_contracts::contracts::CompactionOutput =
        from_json_string(output_json.as_str())?;
    if output.messages.is_empty() && !messages.is_empty() {
        return Err(ProcessModuleError::new(
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
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
) -> Result<ContextBundle, ProcessModuleError> {
    ensure_not_cancelled(host)?;
    let task_json = to_json_string(&input.task)?;
    let bundle_json = match host.build_context_json(String::from(task_json)) {
        Ok(json) => json,
        Err(error) => return Err(ProcessModuleError::new(error.message)),
    };
    from_json_string(bundle_json.as_str())
}

pub(super) fn complete_model(
    host: &mut WorkflowModuleHostMut<'_>,
    request: &CanonicalModelRequest,
    phase: &str,
) -> Result<CanonicalModelResponse, ProcessModuleError> {
    ensure_not_cancelled(host)?;
    let request_json = to_json_string(request)?;
    let response_json = match host.complete_model_json(String::from(request_json)) {
        Ok(json) => json,
        Err(error) => return Err(ProcessModuleError::new(error.message)),
    };
    let response: CanonicalModelResponse = from_json_string(response_json.as_str())?;
    emit_token_usage(host, request, response.usage.clone(), phase)?;
    Ok(response)
}

pub(super) struct SelectedTools {
    pub(super) tools: Vec<ToolSpec>,
    pub(super) metadata: serde_json::Value,
}

impl SelectedTools {
    fn empty() -> Self {
        Self {
            tools: Vec::new(),
            metadata: serde_json::Value::Null,
        }
    }
}

fn visible_tools(
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
    phase: &str,
) -> Result<SelectedTools, ProcessModuleError> {
    ensure_not_cancelled(host)?;
    let request = ToolExposureRequest::new(input.task.clone())
        .with_reason("before_model_request")
        .with_phase(phase);
    let request_json = to_json_string(&request)?;
    let tools_json = match host.select_tools_json(String::from(request_json)) {
        Ok(json) => json,
        Err(error) => return Err(ProcessModuleError::new(error.message)),
    };
    let output: proteus_contracts::contracts::ToolExposureOutput =
        from_json_string(tools_json.as_str())?;
    Ok(SelectedTools {
        tools: output.tools,
        metadata: output.metadata,
    })
}

/// Выполняет батч tool calls одного ответа модели. Workflow-owned meta-tools
/// динамической экспозиции обрабатываются локально; зарегистрированные tools
/// (включая facade-tool `task`) уходят в host batch API, где core применяет
/// registry/policy/orchestrator и выбирает допустимую concurrency.
pub(super) fn execute_tools(
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
    calls: &[ToolCall],
    phase: &str,
) -> Result<Vec<ToolResult>, ProcessModuleError> {
    if calls.len() <= 1
        || calls
            .iter()
            .any(|call| dynamic_tools::is_meta_tool(&call.name))
    {
        return calls
            .iter()
            .map(|call| execute_or_handle_tool(host, input, call, phase))
            .collect();
    }
    ensure_not_cancelled(host)?;
    let task_json = to_json_string(&input.task)?;
    let calls_json = to_json_string(&calls)?;
    let results_json =
        match host.execute_tools_json(String::from(task_json), String::from(calls_json)) {
            Ok(json) => json,
            Err(error) => return Err(ProcessModuleError::new(error.message)),
        };
    from_json_string(results_json.as_str())
}

/// Codex-compatible dispatch for one response batch. Calls omitted from the
/// exact model request become failed tool results and are fed back into the
/// next round; they never reach the host registry/policy/executor path.
pub(super) fn execute_codex_tools(
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
    calls: &[ToolCall],
    request_tools: &[ToolSpec],
    phase: &str,
) -> Result<Vec<ToolResult>, ProcessModuleError> {
    let visible_names = request_tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    if calls
        .iter()
        .all(|call| visible_names.contains(call.name.as_str()))
    {
        return execute_tools(host, input, calls, phase);
    }

    calls
        .iter()
        .map(|call| {
            if visible_names.contains(call.name.as_str()) {
                execute_or_handle_tool(host, input, call, phase)
            } else {
                let kind = match call.surface {
                    proteus_contracts::domain::ToolCallSurface::Freeform => "custom tool call",
                    _ => "call",
                };
                Ok(ToolResult::error(
                    call.id.clone(),
                    format!("unsupported {kind}: {}", call.name),
                ))
            }
        })
        .collect()
}

pub(super) fn execute_tool(
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
    call: &ToolCall,
) -> Result<ToolResult, ProcessModuleError> {
    ensure_not_cancelled(host)?;
    let task_json = to_json_string(&input.task)?;
    let call_json = to_json_string(call)?;
    let result_json = match host.execute_tool_json(String::from(task_json), String::from(call_json))
    {
        Ok(json) => json,
        Err(error) => return Err(ProcessModuleError::new(error.message)),
    };
    from_json_string(result_json.as_str())
}

fn ensure_not_cancelled(host: &mut WorkflowModuleHostMut<'_>) -> Result<(), ProcessModuleError> {
    match host.is_cancelled() {
        Ok(false) => Ok(()),
        Ok(true) => Err(ProcessModuleError::new("turn canceled by client")),
        Err(error) => Err(ProcessModuleError::new(error.message)),
    }
}

pub(super) fn emit_event(
    host: &mut WorkflowModuleHostMut<'_>,
    event: &Event,
) -> Result<(), ProcessModuleError> {
    let event_json = to_json_string(event)?;
    match host.emit_event_json(String::from(event_json)) {
        Ok(()) => Ok(()),
        Err(error) => Err(ProcessModuleError::new(error.message)),
    }
}

pub(super) fn to_json_string<T: serde::Serialize>(value: &T) -> Result<String, ProcessModuleError> {
    serde_json::to_string(value).map_err(|error| ProcessModuleError::new(error.to_string()))
}

pub(super) fn from_json_string<T: serde::de::DeserializeOwned>(
    value: &str,
) -> Result<T, ProcessModuleError> {
    serde_json::from_str(value).map_err(|error| ProcessModuleError::new(error.to_string()))
}

fn emit_token_usage(
    host: &mut WorkflowModuleHostMut<'_>,
    request: &CanonicalModelRequest,
    actual: Option<TokenUsage>,
    phase: &str,
) -> Result<(), ProcessModuleError> {
    let usage = request_token_usage_snapshot(request, actual, phase);
    emit_event(host, &Event::TokenUsageUpdated { usage })
}
