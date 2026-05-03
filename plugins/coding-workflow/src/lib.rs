//! Coding workflow plugin.
//!
//! Owns workflow control-flow, but every runtime capability goes through the
//! narrow workflow host API: context build, model completion, tool visibility,
//! tool execution, and event emission.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use agent_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    contracts::{CompactionInput, ToolExposureRequest},
    domain::{
        AgentOutput, ContextBundle, Event, TokenUsageCategory, TokenUsageSnapshot, ToolCall,
        ToolChoice, ToolResult, ToolSpec,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        InstructionBlock, InstructionKind, MessageRole, TokenUsage,
    },
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginWorkflow,
        PluginWorkflow_TO, PluginWorkflowError, PluginWorkflowHostMut, PluginWorkflowInput,
        PluginWorkflowOutput, WorkflowObject,
    },
};
use serde_json::{Value, json};

const SINGLE_LOOP_MODULE_ID: &str = "coding.single_loop";
const PLAN_EXECUTE_REVIEW_MODULE_ID: &str = "coding.plan_execute_review";
const MAX_TOOL_ROUNDS: usize = 8;
const SYSTEM_INSTRUCTIONS: &str = "You are running inside a modular v0 agent skeleton. Answer normal conversational questions directly. Use tools only when they are necessary and only if they are included in the current tool list.";
const PLAN_SYSTEM_INSTRUCTIONS: &str = "You are running inside a modular coding workflow. First form a concise internal plan, then use tools only when they are necessary, then produce a final answer after reviewing the result.";
const PLAN_DEVELOPER_INSTRUCTIONS: &str = "Planning phase: write a short actionable plan for the user's task. Do not call tools in this phase. Keep it concrete and adjust it later if tool results contradict it.";
const EXECUTE_DEVELOPER_INSTRUCTIONS: &str = "Execute phase: follow the plan, inspect relevant context, and use available tools when they are necessary. If you are ready to answer, provide a concise draft response without calling tools.";
const REVIEW_DEVELOPER_INSTRUCTIONS: &str = "Review phase: produce the final user-facing answer. Mention what changed or what you found, and call out verification gaps if no verification was possible. Do not request tools in this phase.";

pub struct CodingSingleLoopWorkflow {
    pub max_tool_rounds: usize,
}

impl Default for CodingSingleLoopWorkflow {
    fn default() -> Self {
        Self {
            max_tool_rounds: MAX_TOOL_ROUNDS,
        }
    }
}
pub struct CodingPlanExecuteReviewWorkflow;

impl PluginWorkflow for CodingSingleLoopWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        let input: PluginWorkflowInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_single_loop(input, host, self.max_tool_rounds) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => RResult::RErr(error),
        }
    }
}

impl PluginWorkflow for CodingPlanExecuteReviewWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        let input: PluginWorkflowInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_plan_execute_review(input, host) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => RResult::RErr(error),
        }
    }
}

fn run_single_loop(
    input: PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    max_tool_rounds: usize,
) -> Result<PluginWorkflowOutput, PluginWorkflowError> {
    emit_event(
        host,
        &Event::TaskReceived {
            task: input.task.clone(),
        },
    )?;

    let bundle = build_context(host, &input)?;
    emit_event(
        host,
        &Event::ContextBuilt {
            chunks: bundle.chunks.len(),
            token_estimate: bundle.token_estimate,
        },
    )?;

    let context_chunks = bundle.chunks.len();
    let context_token_estimate = bundle.token_estimate;
    let mut persistent_messages = input.history.clone();
    let user_message = CanonicalMessage::text(MessageRole::User, input.task.text.clone());
    persistent_messages.push(user_message.clone());

    let mut model_messages = persistent_messages.clone();
    for chunk in bundle.chunks {
        model_messages.push(
            CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
                .with_name("context"),
        );
    }

    for _round in 0..max_tool_rounds {
        let request = request_from_state(&input, host, &model_messages, SYSTEM_INSTRUCTIONS, None)?;
        emit_event(
            host,
            &Event::ModelRequestPrepared {
                model: request.model.clone(),
            },
        )?;
        let response = complete_model(host, &request, "single_loop")?;
        emit_event(
            host,
            &Event::ModelResponseReceived {
                finish_reason: response.finish_reason.clone(),
            },
        )?;

        model_messages.push(response.message.clone());
        persistent_messages.push(response.message.clone());
        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        if !should_run_tools {
            let output = AgentOutput::new(
                message_text(&response.message),
                output_metadata(
                    SINGLE_LOOP_MODULE_ID,
                    &input,
                    &model_messages,
                    context_chunks,
                    context_token_estimate,
                ),
            );
            emit_event(
                host,
                &Event::TurnFinished {
                    output: output.clone(),
                },
            )?;
            return Ok(PluginWorkflowOutput {
                output,
                messages: persistent_messages,
            });
        }

        for call in response.tool_calls {
            let result = execute_tool(host, &input, &call)?;
            let call_id = result.call_id.clone();
            let tool_result_message =
                CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                    .with_tool_call_id(call_id);
            model_messages.push(tool_result_message.clone());
            persistent_messages.push(tool_result_message);
        }
    }

    let mut request = request_from_state(&input, host, &model_messages, SYSTEM_INSTRUCTIONS, None)?;
    request.tools.clear();
    request.tool_choice = ToolChoice::None;
    emit_event(
        host,
        &Event::ModelRequestPrepared {
            model: request.model.clone(),
        },
    )?;
    let response = complete_model(host, &request, "single_loop_final")?;
    emit_event(
        host,
        &Event::ModelResponseReceived {
            finish_reason: response.finish_reason.clone(),
        },
    )?;

    model_messages.push(response.message.clone());
    persistent_messages.push(response.message.clone());
    let output = AgentOutput::new(
        message_text(&response.message),
        output_metadata_with_extra(
            SINGLE_LOOP_MODULE_ID,
            &input,
            &model_messages,
            context_chunks,
            context_token_estimate,
            json!({
                "max_tool_rounds": max_tool_rounds,
                "tool_round_limit_reached": true,
            }),
        ),
    );
    emit_event(
        host,
        &Event::TurnFinished {
            output: output.clone(),
        },
    )?;
    Ok(PluginWorkflowOutput {
        output,
        messages: persistent_messages,
    })
}

fn run_plan_execute_review(
    input: PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
) -> Result<PluginWorkflowOutput, PluginWorkflowError> {
    emit_event(
        host,
        &Event::TaskReceived {
            task: input.task.clone(),
        },
    )?;

    let bundle = build_context(host, &input)?;
    emit_event(
        host,
        &Event::ContextBuilt {
            chunks: bundle.chunks.len(),
            token_estimate: bundle.token_estimate,
        },
    )?;

    let context_chunks = bundle.chunks.len();
    let context_token_estimate = bundle.token_estimate;
    let mut persistent_messages = input.history.clone();
    let user_message = CanonicalMessage::text(MessageRole::User, input.task.text.clone());
    persistent_messages.push(user_message.clone());

    let mut model_messages = persistent_messages.clone();
    for chunk in bundle.chunks {
        model_messages.push(
            CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
                .with_name("context"),
        );
    }

    let mut plan_request =
        CanonicalModelRequest::new(input.runtime.model_ref.clone(), model_messages.clone())
            .with_instructions(vec![
                InstructionBlock::new(InstructionKind::System, PLAN_SYSTEM_INSTRUCTIONS, 100),
                InstructionBlock::new(InstructionKind::Developer, PLAN_DEVELOPER_INSTRUCTIONS, 90),
            ])
            .with_tool_choice(ToolChoice::None);
    plan_request.tools.clear();
    emit_event(
        host,
        &Event::ModelRequestPrepared {
            model: plan_request.model.clone(),
        },
    )?;
    let plan_response = complete_model(host, &plan_request, "plan")?;
    emit_event(
        host,
        &Event::ModelResponseReceived {
            finish_reason: plan_response.finish_reason.clone(),
        },
    )?;
    let plan_message =
        with_workflow_phase(plan_response.message, PLAN_EXECUTE_REVIEW_MODULE_ID, "plan");
    model_messages.push(plan_message.clone());

    let mut draft_finish_reason = None;
    let mut tool_round_limit_reached = true;
    for _round in 0..MAX_TOOL_ROUNDS {
        let request = request_from_state(
            &input,
            host,
            &model_messages,
            PLAN_SYSTEM_INSTRUCTIONS,
            Some(EXECUTE_DEVELOPER_INSTRUCTIONS),
        )?;
        emit_event(
            host,
            &Event::ModelRequestPrepared {
                model: request.model.clone(),
            },
        )?;
        let response = complete_model(host, &request, "execute")?;
        emit_event(
            host,
            &Event::ModelResponseReceived {
                finish_reason: response.finish_reason.clone(),
            },
        )?;

        let finish_reason = response.finish_reason.clone();
        model_messages.push(response.message.clone());
        persistent_messages.push(response.message.clone());
        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        if !should_run_tools {
            draft_finish_reason = Some(finish_reason);
            tool_round_limit_reached = false;
            break;
        }

        for call in response.tool_calls {
            let result = execute_tool(host, &input, &call)?;
            let call_id = result.call_id.clone();
            let tool_result_message =
                CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                    .with_tool_call_id(call_id);
            model_messages.push(tool_result_message.clone());
            persistent_messages.push(tool_result_message);
        }
    }

    let mut review_request =
        CanonicalModelRequest::new(input.runtime.model_ref.clone(), model_messages.clone())
            .with_instructions(vec![
                InstructionBlock::new(InstructionKind::System, PLAN_SYSTEM_INSTRUCTIONS, 100),
                InstructionBlock::new(
                    InstructionKind::Developer,
                    REVIEW_DEVELOPER_INSTRUCTIONS,
                    90,
                ),
            ])
            .with_tool_choice(ToolChoice::None);
    review_request.tools.clear();
    emit_event(
        host,
        &Event::ModelRequestPrepared {
            model: review_request.model.clone(),
        },
    )?;
    let final_response = complete_model(host, &review_request, "review")?;
    emit_event(
        host,
        &Event::ModelResponseReceived {
            finish_reason: final_response.finish_reason.clone(),
        },
    )?;

    model_messages.push(final_response.message.clone());
    persistent_messages.push(final_response.message.clone());
    let output = AgentOutput::new(
        message_text(&final_response.message),
        output_metadata_with_extra(
            PLAN_EXECUTE_REVIEW_MODULE_ID,
            &input,
            &model_messages,
            context_chunks,
            context_token_estimate,
            json!({
                "max_tool_rounds": MAX_TOOL_ROUNDS,
                "tool_round_limit_reached": tool_round_limit_reached,
                "draft_finish_reason": draft_finish_reason,
                "phases": ["plan", "execute", "review"],
            }),
        ),
    );
    emit_event(
        host,
        &Event::TurnFinished {
            output: output.clone(),
        },
    )?;
    Ok(PluginWorkflowOutput {
        output,
        messages: persistent_messages,
    })
}

fn request_from_state(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
    system_instructions: &str,
    developer_instructions: Option<&str>,
) -> Result<CanonicalModelRequest, PluginWorkflowError> {
    let tools = visible_tools(host, input)?;
    let mut instructions = vec![InstructionBlock::new(
        InstructionKind::System,
        system_instructions,
        100,
    )];
    if let Some(developer_instructions) = developer_instructions {
        instructions.push(InstructionBlock::new(
            InstructionKind::Developer,
            developer_instructions,
            90,
        ));
    }
    let messages = compact_messages(input, host, messages, "before_model_request")?;
    Ok(
        CanonicalModelRequest::new(input.runtime.model_ref.clone(), messages)
            .with_instructions(instructions)
            .with_tools(tools),
    )
}

fn compact_messages(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
    reason: &str,
) -> Result<Vec<CanonicalMessage>, PluginWorkflowError> {
    let compaction_input = CompactionInput::new(
        input.task.clone(),
        input.runtime.model_ref.clone(),
        messages.to_vec(),
    )
    .with_reason(reason)
    .with_token_estimate(estimate_message_tokens(messages));
    let input_json = to_json_string(&compaction_input)?;
    let output_json = match host.compact_history_json(RString::from(input_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    let output: agent_contracts::contracts::CompactionOutput =
        from_json_string(output_json.as_str())?;
    if output.messages.is_empty() && !messages.is_empty() {
        return Err(PluginWorkflowError::new(
            "compactor returned empty messages for non-empty history",
        ));
    }
    Ok(output.messages)
}

fn build_context(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
) -> Result<ContextBundle, PluginWorkflowError> {
    let task_json = to_json_string(&input.task)?;
    let bundle_json = match host.build_context_json(RString::from(task_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    from_json_string(bundle_json.as_str())
}

fn complete_model(
    host: &mut PluginWorkflowHostMut<'_>,
    request: &CanonicalModelRequest,
    phase: &str,
) -> Result<CanonicalModelResponse, PluginWorkflowError> {
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
    let request = ToolExposureRequest::new(input.task.clone()).with_reason("before_model_request");
    let request_json = to_json_string(&request)?;
    let tools_json = match host.select_tools_json(RString::from(request_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    let output: agent_contracts::contracts::ToolExposureOutput =
        from_json_string(tools_json.as_str())?;
    Ok(output.tools)
}

fn execute_tool(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
) -> Result<ToolResult, PluginWorkflowError> {
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

fn emit_event(
    host: &mut PluginWorkflowHostMut<'_>,
    event: &Event,
) -> Result<(), PluginWorkflowError> {
    let event_json = to_json_string(event)?;
    match host.emit_event_json(RString::from(event_json)) {
        RResult::ROk(()) => Ok(()),
        RResult::RErr(error) => Err(PluginWorkflowError::new(error.message.into_string())),
    }
}

fn to_json_string<T: serde::Serialize>(value: &T) -> Result<String, PluginWorkflowError> {
    serde_json::to_string(value).map_err(|error| PluginWorkflowError::new(error.to_string()))
}

fn from_json_string<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, PluginWorkflowError> {
    serde_json::from_str(value).map_err(|error| PluginWorkflowError::new(error.to_string()))
}

fn output_metadata(
    module_id: &str,
    input: &PluginWorkflowInput,
    messages: &[CanonicalMessage],
    context_chunks: usize,
    context_token_estimate: Option<u32>,
) -> Value {
    output_metadata_with_extra(
        module_id,
        input,
        messages,
        context_chunks,
        context_token_estimate,
        json!({}),
    )
}

fn output_metadata_with_extra(
    module_id: &str,
    input: &PluginWorkflowInput,
    messages: &[CanonicalMessage],
    context_chunks: usize,
    context_token_estimate: Option<u32>,
    extra: Value,
) -> Value {
    let token_estimate = estimate_message_tokens(messages).or(context_token_estimate);
    let mut metadata = json!({
        "session_id": input.runtime.session_id,
        "thread_id": input.runtime.thread_id,
        "turn_id": input.runtime.turn_id,
        "model": {
            "provider": input.runtime.model_ref.provider.clone(),
            "name": input.runtime.model_ref.model.clone(),
        },
        "context": {
            "chunks": context_chunks,
            "token_estimate": token_estimate,
            "initial_token_estimate": context_token_estimate,
        },
        "workflow": {
            "source": "plugin",
            "module_id": module_id,
        },
    });

    if let (Value::Object(metadata), Value::Object(extra)) = (&mut metadata, extra) {
        metadata.extend(extra);
    }

    metadata
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

fn request_token_usage_snapshot(
    request: &CanonicalModelRequest,
    actual: Option<TokenUsage>,
    phase: &str,
) -> TokenUsageSnapshot {
    let categories = estimate_request_categories(request);
    let estimated_input_tokens = categories.iter().map(|category| category.tokens).sum();
    TokenUsageSnapshot::new(request.model.clone(), estimated_input_tokens, categories)
        .with_phase(phase)
        .with_actual(actual)
}

fn estimate_request_categories(request: &CanonicalModelRequest) -> Vec<TokenUsageCategory> {
    let mut instructions_bytes = request
        .instructions
        .iter()
        .map(|instruction| instruction.text.len())
        .sum::<usize>();
    if !request.instructions.is_empty() {
        instructions_bytes += request.instructions.len() * 8;
    }

    let mut message_bytes = 0usize;
    let mut context_bytes = 0usize;
    let mut tool_result_bytes = 0usize;
    let mut file_bytes = 0usize;
    for message in &request.messages {
        message_bytes += 4;
        for part in &message.parts {
            match part {
                ContentPart::Text { text } | ContentPart::ReasoningSummary { text } => {
                    message_bytes += text.len();
                }
                ContentPart::Context { chunk } => {
                    context_bytes += chunk.source.len()
                        + chunk
                            .path
                            .as_ref()
                            .map(|path| path.display().to_string().len())
                            .unwrap_or_default()
                        + chunk.content.len()
                        + chunk.metadata.to_string().len();
                }
                ContentPart::FileRef { path, content } => {
                    file_bytes += path.display().to_string().len()
                        + content.as_deref().unwrap_or_default().len();
                }
                ContentPart::ToolCall { call } => {
                    message_bytes += call.name.len() + call.args.to_string().len();
                }
                ContentPart::ToolResult { result } => {
                    tool_result_bytes += result.output.len()
                        + result.error.as_deref().unwrap_or_default().len()
                        + result.metadata.to_string().len()
                        + result
                            .content
                            .iter()
                            .map(tool_content_text_len)
                            .sum::<usize>();
                }
                ContentPart::Patch { patch } => {
                    message_bytes += patch.content.len();
                }
                _ => {}
            }
        }
    }

    let tool_schema_bytes = request
        .tools
        .iter()
        .map(|tool| {
            serde_json::to_string(tool)
                .map(|json| json.len())
                .unwrap_or(0)
        })
        .sum::<usize>();

    [
        ("instructions", instructions_bytes),
        ("messages", message_bytes),
        ("context", context_bytes),
        ("tool_results", tool_result_bytes),
        ("files", file_bytes),
        ("tool_schemas", tool_schema_bytes),
    ]
    .into_iter()
    .filter_map(|(name, bytes)| {
        let tokens = estimate_tokens_from_bytes(bytes);
        (tokens > 0).then(|| TokenUsageCategory::new(name, tokens))
    })
    .collect()
}

fn estimate_tokens_from_bytes(bytes: usize) -> u32 {
    if bytes == 0 {
        0
    } else {
        (bytes / 4).max(1) as u32
    }
}

fn tool_content_text_len(content: &agent_contracts::domain::ToolContent) -> usize {
    match content {
        agent_contracts::domain::ToolContent::Text { text } => text.len(),
        agent_contracts::domain::ToolContent::Json { value } => value.to_string().len(),
        agent_contracts::domain::ToolContent::Image { data, .. }
        | agent_contracts::domain::ToolContent::Binary { data, .. } => data.len(),
        _ => 0,
    }
}

fn with_workflow_phase(
    mut message: CanonicalMessage,
    module_id: &str,
    phase: &str,
) -> CanonicalMessage {
    match &mut message.metadata {
        Value::Object(metadata) => {
            metadata.insert(
                "workflow_module".to_owned(),
                Value::String(module_id.to_owned()),
            );
            metadata.insert("workflow_phase".to_owned(), Value::String(phase.to_owned()));
        }
        Value::Null => {
            message.metadata = json!({
                "workflow_module": module_id,
                "workflow_phase": phase,
            });
        }
        other => {
            let previous = std::mem::replace(other, Value::Null);
            message.metadata = json!({
                "workflow_module": module_id,
                "workflow_phase": phase,
                "previous_metadata": previous,
            });
        }
    }
    message
}

fn estimate_message_tokens(messages: &[CanonicalMessage]) -> Option<u32> {
    let bytes = messages
        .iter()
        .flat_map(|message| &message.parts)
        .map(part_text_len)
        .sum::<usize>();
    Some((bytes / 4 + messages.len()).max(1) as u32)
}

fn part_text_len(part: &ContentPart) -> usize {
    match part {
        ContentPart::Text { text } => text.len(),
        ContentPart::Context { chunk } => chunk.content.len(),
        ContentPart::FileRef { content, .. } => content.as_deref().unwrap_or_default().len(),
        ContentPart::ToolCall { call } => call.name.len() + call.args.to_string().len(),
        ContentPart::ToolResult { result } => {
            result.output.len()
                + result.error.as_deref().unwrap_or_default().len()
                + result.metadata.to_string().len()
        }
        ContentPart::Patch { patch } => patch.content.len(),
        ContentPart::ReasoningSummary { text } => text.len(),
        _ => 0,
    }
}

fn message_text(message: &CanonicalMessage) -> String {
    let text = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        "<empty model response>".to_owned()
    } else {
        text
    }
}

fn workflow_err<T>(error: impl ToString) -> RResult<T, PluginWorkflowError> {
    RResult::RErr(PluginWorkflowError::new(error.to_string()))
}

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let workflow: WorkflowObject =
        PluginWorkflow_TO::from_value(CodingSingleLoopWorkflow::default(), TD_Opaque);
    if let RResult::RErr(err) =
        registry.register_workflow(RString::from(SINGLE_LOOP_MODULE_ID), workflow)
    {
        return RResult::RErr(err);
    }

    let plan_workflow: WorkflowObject =
        PluginWorkflow_TO::from_value(CodingPlanExecuteReviewWorkflow, TD_Opaque);
    registry.register_workflow(RString::from(PLAN_EXECUTE_REVIEW_MODULE_ID), plan_workflow)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("coding-workflow"),
        description: RStr::from_str(
            "Workflow plugin providing coding.single_loop and coding.plan_execute_review through the workflow host API",
        ),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    use agent_contracts::{
        abi_stable::sabi_trait::TD_Opaque,
        domain::{AgentTask, ContextChunk, ModelRef, new_session_id, new_thread_id, new_turn_id},
        plugin::{
            PluginWorkflowHost, PluginWorkflowHost_TO, PluginWorkflowHostError,
            PluginWorkflowRuntimeInfo,
        },
    };

    #[test]
    fn empty_text_response_gets_placeholder() {
        let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

        assert_eq!(message_text(&message), "<empty model response>");
    }

    #[test]
    fn estimates_tokens_from_text_context_and_tool_results() {
        let result =
            ToolResult::ok(agent_contracts::domain::new_call_id(), "abcd").with_metadata(json!({}));
        let messages = vec![
            CanonicalMessage::text(MessageRole::User, "abcd"),
            CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }]),
        ];

        assert_eq!(estimate_message_tokens(&messages), Some(4));
    }

    #[derive(Default)]
    struct FakeHost {
        events: Mutex<Vec<Event>>,
        requests: Mutex<Vec<CanonicalModelRequest>>,
        responses: Mutex<VecDeque<CanonicalModelResponse>>,
    }

    impl FakeHost {
        fn with_responses(responses: Vec<CanonicalModelResponse>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    impl PluginWorkflowHost for FakeHost {
        fn build_context_json(
            &self,
            task_json: RString,
        ) -> RResult<RString, PluginWorkflowHostError> {
            let task: AgentTask = serde_json::from_str(task_json.as_str()).expect("task json");
            let bundle = ContextBundle::new(vec![ContextChunk::new(
                "test",
                format!("context for {}", task.text),
            )])
            .with_token_estimate(7);
            RResult::ROk(RString::from(
                serde_json::to_string(&bundle).expect("bundle json"),
            ))
        }

        fn complete_model_json(
            &self,
            request_json: RString,
        ) -> RResult<RString, PluginWorkflowHostError> {
            let request: CanonicalModelRequest =
                serde_json::from_str(request_json.as_str()).expect("request json");
            self.requests.lock().expect("requests").push(request);
            let response = self
                .responses
                .lock()
                .expect("responses")
                .pop_front()
                .unwrap_or_else(|| {
                    CanonicalModelResponse::new(
                        CanonicalMessage::text(MessageRole::Assistant, "done"),
                        Vec::new(),
                        FinishReason::Stop,
                    )
                });
            RResult::ROk(RString::from(
                serde_json::to_string(&response).expect("response json"),
            ))
        }

        fn compact_history_json(
            &self,
            input_json: RString,
        ) -> RResult<RString, PluginWorkflowHostError> {
            let input: CompactionInput =
                serde_json::from_str(input_json.as_str()).expect("compaction input json");
            let output = agent_contracts::contracts::CompactionOutput::unchanged(input.messages);
            RResult::ROk(RString::from(
                serde_json::to_string(&output).expect("compaction output json"),
            ))
        }

        fn visible_tools_json(&self, _cwd: RString) -> RResult<RString, PluginWorkflowHostError> {
            RResult::ROk(RString::from("[]"))
        }

        fn select_tools_json(
            &self,
            _request_json: RString,
        ) -> RResult<RString, PluginWorkflowHostError> {
            let output = agent_contracts::contracts::ToolExposureOutput::new(Vec::new());
            RResult::ROk(RString::from(
                serde_json::to_string(&output).expect("tool exposure output json"),
            ))
        }

        fn execute_tool_json(
            &self,
            _task_json: RString,
            _call_json: RString,
        ) -> RResult<RString, PluginWorkflowHostError> {
            RResult::RErr(PluginWorkflowHostError::new("unexpected tool call"))
        }

        fn emit_event_json(&self, event_json: RString) -> RResult<(), PluginWorkflowHostError> {
            let event: Event = serde_json::from_str(event_json.as_str()).expect("event json");
            self.events.lock().expect("events").push(event);
            RResult::ROk(())
        }
    }

    #[test]
    fn single_loop_calls_host_and_returns_persistent_messages() {
        let input = PluginWorkflowInput {
            task: AgentTask::new("hello", std::env::current_dir().expect("cwd")),
            history: Vec::new(),
            runtime: PluginWorkflowRuntimeInfo {
                session_id: new_session_id(),
                thread_id: new_thread_id(),
                turn_id: new_turn_id(),
                model_ref: ModelRef::new("fake", "model"),
                model_timeout_ms: 120_000,
                context_timeout_ms: 30_000,
            },
        };
        let input_json = serde_json::to_string(&input).expect("input json");
        let mut host = FakeHost::default();
        let mut host_to: PluginWorkflowHostMut<'_> =
            PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

        let output_json = match CodingSingleLoopWorkflow::default()
            .run_json(RString::from(input_json), &mut host_to)
        {
            RResult::ROk(json) => json,
            RResult::RErr(error) => panic!("workflow failed: {}", error.message),
        };
        let output: PluginWorkflowOutput =
            serde_json::from_str(output_json.as_str()).expect("output json");
        drop(host_to);

        assert_eq!(output.output.text, "done");
        assert_eq!(
            output.output.metadata["workflow"]["module_id"],
            SINGLE_LOOP_MODULE_ID
        );
        assert_eq!(output.messages.len(), 2);

        let requests = host.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].tools.len(), 0);
        assert!(
            requests[0]
                .messages
                .iter()
                .any(|message| message.name.as_deref() == Some("context"))
        );

        let events = host.events.lock().expect("events");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::TaskReceived { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::ContextBuilt { chunks: 1, .. }))
        );
        assert!(events.iter().any(|event| matches!(
            event,
            Event::TokenUsageUpdated {
                usage
            } if usage.categories.iter().any(|category| category.name == "context")
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::TurnFinished { .. }))
        );
    }

    #[test]
    fn plan_execute_review_runs_plan_execute_and_review_requests() {
        let input = PluginWorkflowInput {
            task: AgentTask::new("change code", std::env::current_dir().expect("cwd")),
            history: Vec::new(),
            runtime: PluginWorkflowRuntimeInfo {
                session_id: new_session_id(),
                thread_id: new_thread_id(),
                turn_id: new_turn_id(),
                model_ref: ModelRef::new("fake", "model"),
                model_timeout_ms: 120_000,
                context_timeout_ms: 30_000,
            },
        };
        let input_json = serde_json::to_string(&input).expect("input json");
        let mut host = FakeHost::with_responses(vec![
            CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, "plan"),
                Vec::new(),
                FinishReason::Stop,
            ),
            CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, "draft"),
                Vec::new(),
                FinishReason::Stop,
            ),
            CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, "final"),
                Vec::new(),
                FinishReason::Stop,
            ),
        ]);
        let mut host_to: PluginWorkflowHostMut<'_> =
            PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

        let output_json = match CodingPlanExecuteReviewWorkflow
            .run_json(RString::from(input_json), &mut host_to)
        {
            RResult::ROk(json) => json,
            RResult::RErr(error) => panic!("workflow failed: {}", error.message),
        };
        let output: PluginWorkflowOutput =
            serde_json::from_str(output_json.as_str()).expect("output json");
        drop(host_to);

        assert_eq!(output.output.text, "final");
        assert_eq!(
            output.output.metadata["workflow"]["module_id"],
            PLAN_EXECUTE_REVIEW_MODULE_ID
        );
        assert_eq!(
            output.output.metadata["phases"],
            json!(["plan", "execute", "review"])
        );
        assert_eq!(output.messages.len(), 3);
        assert!(
            output
                .messages
                .iter()
                .all(|message| message.metadata["workflow_phase"] != "plan")
        );

        let requests = host.requests.lock().expect("requests");
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].tool_choice, ToolChoice::None);
        assert_eq!(requests[0].tools.len(), 0);
        assert_eq!(requests[2].tool_choice, ToolChoice::None);
        assert_eq!(requests[2].tools.len(), 0);
    }
}
