//! Coding workflow plugin.
//!
//! Owns workflow control-flow, but every runtime capability goes through the
//! narrow workflow host API: context build, model completion, tool visibility,
//! tool execution, and event emission.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

mod dynamic_tools;

use std::collections::HashSet;

use proteus_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    contracts::{CompactionInput, ToolExposureRequest},
    domain::{
        AgentOutput, CacheHints, ContextBundle, Event, HistoryCompactionReport, TokenUsageCategory,
        TokenUsageSnapshot, TokenUsageSource, ToolCall, ToolChoice, ToolResult, ToolSafety,
        ToolSpec,
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
const CODEX_LOOP_MODULE_ID: &str = "coding.codex_loop";
const CODEX_LOOP_DIAGNOSTIC_MODULE_ID: &str = "coding.codex_loop_diagnostic";
const PLAN_EXECUTE_REVIEW_MODULE_ID: &str = "coding.plan_execute_review";
const MAX_TOOL_ROUNDS: usize = 8;
const SYSTEM_INSTRUCTIONS: &str = "\
You are running inside a modular v0 agent skeleton. Answer normal conversational \
questions directly. Use tools only when they are necessary and only if they are \
included in the current tool list. If the user says they are testing the agent \
or tools, focus on the requested test and do not inspect the project unless \
asked. Do not call remember_fact for temporary test notes; use it only when the \
user explicitly asks you to remember a stable preference or durable project fact. \
Do not invent dates or times; omit them unless the user supplied them or you \
verified them with a tool.";
const PLAN_SYSTEM_INSTRUCTIONS: &str = "\
You are running inside a modular coding workflow. First form a concise internal \
plan, then use tools only when they are necessary, then produce a final answer \
after reviewing the result. If the user says they are testing the agent or tools, \
focus on the requested test and do not inspect the project unless asked. Do not \
call remember_fact for temporary test notes; use it only when the user explicitly \
asks you to remember a stable preference or durable project fact. Do not invent \
dates or times; omit them unless the user supplied them or you verified them with \
a tool.";
const PLAN_DEVELOPER_INSTRUCTIONS: &str = "\
Interview-first planning phase: clarify material requirements before writing \
the final plan. You may use read-only tools to discover facts. For broad or \
underspecified tasks, call request_user_input with one focused multiple-choice \
question before writing a staged plan; ask follow-up questions only after prior \
answers when the next question depends on them. If all material requirements \
are already clear, produce a concise actionable plan. Do not ask whether the \
plan is approved; the client handles approval after the final plan. Do not use \
write, shell, network, or mutation-oriented tools in this phase.";
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
pub struct CodingCodexLoopWorkflow;
pub struct CodingCodexLoopDiagnosticWorkflow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmptyFinalResponseMode {
    Strict,
    LastToolResultDiagnostic,
}

struct PreparedRequest {
    request: CanonicalModelRequest,
    compaction: Option<HistoryCompactionReport>,
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

impl PluginWorkflow for CodingCodexLoopWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        let input: PluginWorkflowInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_codex_loop(
            input,
            host,
            CODEX_LOOP_MODULE_ID,
            EmptyFinalResponseMode::Strict,
        ) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => RResult::RErr(error),
        }
    }
}

impl PluginWorkflow for CodingCodexLoopDiagnosticWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        let input: PluginWorkflowInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_codex_loop(
            input,
            host,
            CODEX_LOOP_DIAGNOSTIC_MODULE_ID,
            EmptyFinalResponseMode::LastToolResultDiagnostic,
        ) {
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
    let mut compactions = Vec::new();
    let mut persistent_messages = input.history.clone();
    let user_message = CanonicalMessage::text(MessageRole::User, input.task.text.clone());
    let current_user_message_id = user_message.id;
    persistent_messages.push(user_message.clone());

    let mut model_messages = persistent_messages.clone();
    for chunk in bundle.chunks {
        model_messages.push(
            CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
                .with_name("context"),
        );
    }
    let mut current_turn_messages_start = model_messages.len();

    for _round in 0..max_tool_rounds {
        let prepared = request_from_state(
            &input,
            host,
            &model_messages,
            SYSTEM_INSTRUCTIONS,
            None,
            "single_loop",
        )?;
        if let Some(report) = prepared.compaction.as_ref() {
            compactions.push(report.clone());
            if report.changed {
                model_messages = prepared.request.messages.clone();
                persistent_messages = persistent_messages_from_model_messages(&model_messages);
                current_turn_messages_start =
                    current_turn_start(&model_messages, current_user_message_id);
            }
        }
        let request = prepared.request;
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
                output_text(
                    &response.message,
                    &model_messages[current_turn_messages_start..],
                ),
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
            let new_messages_start =
                current_turn_start(&persistent_messages, current_user_message_id);
            return Ok(PluginWorkflowOutput {
                output,
                messages: persistent_messages,
                new_messages_start: Some(new_messages_start),
                compactions,
            });
        }

        for call in response.tool_calls {
            let result = execute_or_handle_tool(host, &input, &call, "single_loop")?;
            let call_id = result.call_id.clone();
            let tool_result_message =
                CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                    .with_tool_call_id(call_id);
            model_messages.push(tool_result_message.clone());
            persistent_messages.push(tool_result_message);
        }
    }

    let prepared = request_from_state(
        &input,
        host,
        &model_messages,
        SYSTEM_INSTRUCTIONS,
        None,
        "single_loop_final",
    )?;
    if let Some(report) = prepared.compaction.as_ref() {
        compactions.push(report.clone());
        if report.changed {
            model_messages = prepared.request.messages.clone();
            persistent_messages = persistent_messages_from_model_messages(&model_messages);
            current_turn_messages_start =
                current_turn_start(&model_messages, current_user_message_id);
        }
    }
    let mut request = prepared.request;
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
        output_text(
            &response.message,
            &model_messages[current_turn_messages_start..],
        ),
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
    let new_messages_start = current_turn_start(&persistent_messages, current_user_message_id);
    Ok(PluginWorkflowOutput {
        output,
        messages: persistent_messages,
        new_messages_start: Some(new_messages_start),
        compactions,
    })
}

fn run_codex_loop(
    input: PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    module_id: &str,
    empty_final_response_mode: EmptyFinalResponseMode,
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
    let mut compactions = Vec::new();
    let mut persistent_messages = input.history.clone();
    let user_message = CanonicalMessage::text(MessageRole::User, input.task.text.clone());
    let current_user_message_id = user_message.id;
    persistent_messages.push(user_message.clone());

    let mut model_messages = persistent_messages.clone();
    for chunk in bundle.chunks {
        model_messages.push(
            CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
                .with_name("context"),
        );
    }
    let mut current_turn_messages_start = model_messages.len();
    let mut tool_rounds = 0usize;
    let mut executed_tools = Vec::new();

    loop {
        let prepared = request_from_state_with_instruction_blocks(
            &input,
            host,
            &model_messages,
            input.runtime.instructions.clone(),
            None,
            "codex_loop",
        )?;
        if let Some(report) = prepared.compaction.as_ref() {
            compactions.push(report.clone());
            if report.changed {
                replace_after_compaction(
                    &prepared.request.messages,
                    &mut model_messages,
                    &mut persistent_messages,
                    current_user_message_id,
                    &[],
                )?;
                current_turn_messages_start =
                    current_turn_start(&model_messages, current_user_message_id);
            }
        }
        let request = prepared.request;
        emit_event(
            host,
            &Event::ModelRequestPrepared {
                model: request.model.clone(),
            },
        )?;
        let response = complete_model(host, &request, "codex_loop")?;
        emit_event(
            host,
            &Event::ModelResponseReceived {
                finish_reason: response.finish_reason.clone(),
            },
        )?;
        validate_codex_model_response(&request, &response)?;

        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        let assistant_message = response.message.clone();
        model_messages.push(assistant_message.clone());
        persistent_messages.push(assistant_message.clone());

        if should_run_tools {
            tool_rounds += 1;
            for call in response.tool_calls {
                executed_tools.push(call.name.clone());
                let result = execute_or_handle_tool(host, &input, &call, "codex_loop")?;
                let call_id = result.call_id.clone();
                let tool_result_message = CanonicalMessage::new(
                    MessageRole::Tool,
                    vec![ContentPart::ToolResult { result }],
                )
                .with_tool_call_id(call_id);
                model_messages.push(tool_result_message.clone());
                persistent_messages.push(tool_result_message);
            }
            continue;
        }

        let output = AgentOutput::new(
            match empty_final_response_mode {
                EmptyFinalResponseMode::Strict => message_text(&assistant_message),
                EmptyFinalResponseMode::LastToolResultDiagnostic => output_text(
                    &assistant_message,
                    &model_messages[current_turn_messages_start..],
                ),
            },
            output_metadata_with_extra(
                module_id,
                &input,
                &model_messages,
                context_chunks,
                context_token_estimate,
                json!({
                    "tool_rounds": tool_rounds,
                    "phases": ["turn_loop"],
                    "executed_tools": executed_tools,
                }),
            ),
        );
        emit_event(
            host,
            &Event::TurnFinished {
                output: output.clone(),
            },
        )?;
        let new_messages_start = current_turn_start(&persistent_messages, current_user_message_id);
        return Ok(PluginWorkflowOutput {
            output,
            messages: persistent_messages,
            new_messages_start: Some(new_messages_start),
            compactions,
        });
    }
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
    let mut compactions = Vec::new();
    let mut persistent_messages = input.history.clone();
    let user_message = CanonicalMessage::text(MessageRole::User, input.task.text.clone());
    let current_user_message_id = user_message.id;
    persistent_messages.push(user_message.clone());

    let mut model_messages = persistent_messages.clone();
    for chunk in bundle.chunks {
        model_messages.push(
            CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
                .with_name("context"),
        );
    }
    let mut current_turn_messages_start = model_messages.len();

    let prepared = request_from_state(
        &input,
        host,
        &model_messages,
        PLAN_SYSTEM_INSTRUCTIONS,
        Some(PLAN_DEVELOPER_INSTRUCTIONS),
        "plan",
    )?;
    if let Some(report) = prepared.compaction.as_ref() {
        compactions.push(report.clone());
        if report.changed {
            model_messages = prepared.request.messages.clone();
            persistent_messages = persistent_messages_from_model_messages(&model_messages);
            current_turn_messages_start =
                current_turn_start(&model_messages, current_user_message_id);
        }
    }
    let mut plan_request = prepared.request;
    plan_request
        .tools
        .retain(|tool| matches!(tool.safety, ToolSafety::ReadOnly));
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
        let prepared = request_from_state(
            &input,
            host,
            &model_messages,
            PLAN_SYSTEM_INSTRUCTIONS,
            Some(EXECUTE_DEVELOPER_INSTRUCTIONS),
            "execute",
        )?;
        if let Some(report) = prepared.compaction.as_ref() {
            compactions.push(report.clone());
            if report.changed {
                model_messages = prepared.request.messages.clone();
                persistent_messages = persistent_messages_from_model_messages(&model_messages);
                current_turn_messages_start =
                    current_turn_start(&model_messages, current_user_message_id);
            }
        }
        let request = prepared.request;
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
        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        if should_run_tools {
            persistent_messages.push(response.message.clone());
        }
        if !should_run_tools {
            draft_finish_reason = Some(finish_reason);
            tool_round_limit_reached = false;
            break;
        }

        for call in response.tool_calls {
            let result = execute_or_handle_tool(host, &input, &call, "execute")?;
            let call_id = result.call_id.clone();
            let tool_result_message =
                CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                    .with_tool_call_id(call_id);
            model_messages.push(tool_result_message.clone());
            persistent_messages.push(tool_result_message);
        }
    }

    let prepared = request_from_state(
        &input,
        host,
        &model_messages,
        PLAN_SYSTEM_INSTRUCTIONS,
        Some(REVIEW_DEVELOPER_INSTRUCTIONS),
        "review",
    )?;
    if let Some(report) = prepared.compaction.as_ref() {
        compactions.push(report.clone());
        if report.changed {
            model_messages = prepared.request.messages.clone();
            persistent_messages = persistent_messages_from_model_messages(&model_messages);
            current_turn_messages_start =
                current_turn_start(&model_messages, current_user_message_id);
        }
    }
    let mut review_request = prepared.request.with_tool_choice(ToolChoice::None);
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
        output_text(
            &final_response.message,
            &model_messages[current_turn_messages_start..],
        ),
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
    let new_messages_start = current_turn_start(&persistent_messages, current_user_message_id);
    Ok(PluginWorkflowOutput {
        output,
        messages: persistent_messages,
        new_messages_start: Some(new_messages_start),
        compactions,
    })
}

fn request_from_state(
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

fn request_from_state_with_instruction_blocks(
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
    insert_request_metadata_value(
        &mut request,
        "prompt_cache_key",
        json!(prompt_cache_key(input)),
    );
    Ok(PreparedRequest {
        request,
        compaction: compacted.report,
    })
}

fn execute_or_handle_tool(
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

fn validate_codex_model_response(
    request: &CanonicalModelRequest,
    response: &CanonicalModelResponse,
) -> Result<(), PluginWorkflowError> {
    match response.finish_reason {
        FinishReason::ToolCalls if response.tool_calls.is_empty() => {
            return Err(PluginWorkflowError::new(
                "codex_loop model response used finish_reason=ToolCalls without tool calls",
            ));
        }
        FinishReason::ToolCalls => {}
        FinishReason::Stop if response.tool_calls.is_empty() => return Ok(()),
        FinishReason::Length => {
            return Err(PluginWorkflowError::new(
                "codex_loop model response hit the length limit before finishing the turn",
            ));
        }
        FinishReason::ContentFilter | FinishReason::Error | FinishReason::Unknown => {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model response returned non-success finish_reason={:?}",
                response.finish_reason
            )));
        }
        _ if !response.tool_calls.is_empty() => {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model response included tool calls with finish_reason={:?}",
                response.finish_reason
            )));
        }
        _ => return Ok(()),
    }

    validate_tool_calls_match_message(&response.message, &response.tool_calls)?;
    validate_tool_calls_are_request_visible(&request.tools, &response.tool_calls)
}

fn validate_tool_calls_match_message(
    message: &CanonicalMessage,
    tool_calls: &[ToolCall],
) -> Result<(), PluginWorkflowError> {
    let message_tool_calls = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolCall { call } => Some(call),
            _ => None,
        })
        .collect::<Vec<_>>();
    if message_tool_calls.len() != tool_calls.len() {
        return Err(PluginWorkflowError::new(format!(
            "codex_loop model response tool_calls length {} does not match assistant message tool_call parts {}",
            tool_calls.len(),
            message_tool_calls.len()
        )));
    }

    let mut seen_call_ids = HashSet::new();
    for (index, (message_call, response_call)) in
        message_tool_calls.iter().zip(tool_calls.iter()).enumerate()
    {
        if !seen_call_ids.insert(response_call.id.clone()) {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model response duplicated tool call id '{}'",
                response_call.id
            )));
        }
        if *message_call != response_call {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model response tool call at index {index} does not match assistant message part"
            )));
        }
    }

    Ok(())
}

fn validate_tool_calls_are_request_visible(
    request_tools: &[ToolSpec],
    tool_calls: &[ToolCall],
) -> Result<(), PluginWorkflowError> {
    let visible_names = request_tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    for call in tool_calls {
        if !visible_names.contains(call.name.as_str()) {
            return Err(PluginWorkflowError::new(format!(
                "codex_loop model requested tool '{}' that was not present in the model request",
                call.name
            )));
        }
    }
    Ok(())
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

fn build_context(
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

fn complete_model(
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

fn execute_tool(
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

fn model_auto_compact_limit(max_input_tokens: Option<u32>) -> Option<u32> {
    max_input_tokens.map(|tokens| {
        let limit = (u64::from(tokens) * 8 / 10).max(1);
        u32::try_from(limit).unwrap_or(u32::MAX)
    })
}

fn current_turn_start(
    messages: &[CanonicalMessage],
    current_user_message_id: proteus_contracts::domain::MessageId,
) -> usize {
    maybe_current_turn_start(messages, current_user_message_id).unwrap_or(messages.len())
}

fn maybe_current_turn_start(
    messages: &[CanonicalMessage],
    current_user_message_id: proteus_contracts::domain::MessageId,
) -> Option<usize> {
    messages
        .iter()
        .position(|message| message.id == current_user_message_id)
}

fn persistent_messages_from_model_messages(messages: &[CanonicalMessage]) -> Vec<CanonicalMessage> {
    persistent_messages_from_model_messages_excluding_phases(messages, &[])
}

fn persistent_messages_from_model_messages_excluding_phases(
    messages: &[CanonicalMessage],
    excluded_phases: &[&str],
) -> Vec<CanonicalMessage> {
    messages
        .iter()
        .filter(|message| !is_ephemeral_context_message(message))
        .filter(|message| {
            !message
                .metadata
                .get("workflow_phase")
                .and_then(Value::as_str)
                .is_some_and(|phase| excluded_phases.contains(&phase))
        })
        .cloned()
        .collect()
}

fn replace_after_compaction(
    compacted_messages: &[CanonicalMessage],
    model_messages: &mut Vec<CanonicalMessage>,
    persistent_messages: &mut Vec<CanonicalMessage>,
    current_user_message_id: proteus_contracts::domain::MessageId,
    excluded_persistent_phases: &[&str],
) -> Result<usize, PluginWorkflowError> {
    let current_turn_messages_start =
        maybe_current_turn_start(compacted_messages, current_user_message_id).ok_or_else(|| {
            PluginWorkflowError::new(
                "compaction changed history but dropped the current user message",
            )
        })?;
    *model_messages = compacted_messages.to_vec();
    *persistent_messages = persistent_messages_from_model_messages_excluding_phases(
        model_messages,
        excluded_persistent_phases,
    );
    if maybe_current_turn_start(persistent_messages, current_user_message_id).is_none() {
        return Err(PluginWorkflowError::new(
            "compaction changed persistent history but dropped the current user message",
        ));
    }
    Ok(current_turn_messages_start)
}

fn is_ephemeral_context_message(message: &CanonicalMessage) -> bool {
    message.name.as_deref() == Some("context")
        || message
            .parts
            .iter()
            .all(|part| matches!(part, ContentPart::Context { .. }))
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
    let source = if actual.is_some() {
        TokenUsageSource::Mixed
    } else {
        TokenUsageSource::Estimated
    };
    // Порог автокомпакта кладёт в metadata request_from_state по отчёту
    // компактора — берём его, чтобы метка в клиентах совпадала с триггером.
    let compaction_trigger_tokens = request
        .metadata
        .get("compaction_trigger_tokens")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    TokenUsageSnapshot::new(request.model.clone(), estimated_input_tokens, categories)
        .with_phase(phase)
        .with_max_input_tokens(request.limits.max_input_tokens)
        .with_compaction_trigger_tokens(compaction_trigger_tokens)
        .with_actual(actual)
        .with_source(source)
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
                ContentPart::Text { text }
                | ContentPart::ReasoningSummary { text }
                | ContentPart::Reasoning { text, .. } => {
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

fn tool_content_text_len(content: &proteus_contracts::domain::ToolContent) -> usize {
    match content {
        proteus_contracts::domain::ToolContent::Text { text } => text.len(),
        proteus_contracts::domain::ToolContent::Json { value } => value.to_string().len(),
        proteus_contracts::domain::ToolContent::Image { data, .. }
        | proteus_contracts::domain::ToolContent::Binary { data, .. } => data.len(),
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

fn insert_request_metadata_u32(request: &mut CanonicalModelRequest, key: &str, value: u32) {
    insert_request_metadata_value(request, key, json!(value));
}

fn insert_request_metadata_value(request: &mut CanonicalModelRequest, key: &str, value: Value) {
    match &mut request.metadata {
        Value::Object(metadata) => {
            metadata.insert(key.to_owned(), value);
        }
        Value::Null => {
            request.metadata = json!({ key: value });
        }
        other => {
            let previous = std::mem::replace(other, Value::Null);
            request.metadata = json!({
                key: value,
                "previous_metadata": previous,
            });
        }
    }
}

fn prompt_cache_key(input: &PluginWorkflowInput) -> String {
    let model = sanitize_cache_key_component(&input.runtime.model_ref.model);
    let workspace_hash = stable_hash64(input.task.cwd.to_string_lossy().as_bytes());
    format!("proteus:{model}:{workspace_hash:016x}")
}

fn sanitize_cache_key_component(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    out.truncate(64);
    if out.is_empty() {
        "model".to_owned()
    } else {
        out
    }
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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
        ContentPart::ReasoningSummary { text } | ContentPart::Reasoning { text, .. } => text.len(),
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

fn output_text(message: &CanonicalMessage, messages: &[CanonicalMessage]) -> String {
    let text = message_text(message);
    if text != "<empty model response>" {
        return text;
    }

    let Some(result) = latest_tool_result(messages) else {
        return text;
    };
    let summary = tool_result_summary(result);
    if summary.is_empty() {
        return text;
    }
    format!(
        "Model returned an empty final response after the last tool call.\n\nLast tool result:\n{}",
        truncate_chars(&summary, 2_000)
    )
}

fn latest_tool_result(messages: &[CanonicalMessage]) -> Option<&ToolResult> {
    messages.iter().rev().find_map(|message| {
        message.parts.iter().rev().find_map(|part| match part {
            ContentPart::ToolResult { result } => Some(result),
            _ => None,
        })
    })
}

fn tool_result_summary(result: &ToolResult) -> String {
    let mut parts = Vec::new();
    let output = result.output.trim();
    if !output.is_empty() {
        parts.push(output.to_owned());
    }
    if let Some(error) = result
        .error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
    {
        parts.push(error.to_owned());
    }
    parts.join("\n")
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let mut truncated = text.chars().take(limit).collect::<String>();
    if text.chars().count() > limit {
        truncated.push_str("\n[truncated]");
    }
    truncated
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

    let codex_workflow: WorkflowObject =
        PluginWorkflow_TO::from_value(CodingCodexLoopWorkflow, TD_Opaque);
    if let RResult::RErr(err) =
        registry.register_workflow(RString::from(CODEX_LOOP_MODULE_ID), codex_workflow)
    {
        return RResult::RErr(err);
    }

    let codex_diagnostic_workflow: WorkflowObject =
        PluginWorkflow_TO::from_value(CodingCodexLoopDiagnosticWorkflow, TD_Opaque);
    if let RResult::RErr(err) = registry.register_workflow(
        RString::from(CODEX_LOOP_DIAGNOSTIC_MODULE_ID),
        codex_diagnostic_workflow,
    ) {
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
            "Workflow plugin providing coding.single_loop, coding.codex_loop, coding.codex_loop_diagnostic, and coding.plan_execute_review through the workflow host API",
        ),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests;
