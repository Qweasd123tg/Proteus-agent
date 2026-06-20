//! Coding workflow plugin.
//!
//! Owns workflow control-flow, but every runtime capability goes through the
//! narrow workflow host API: context build, model completion, tool visibility,
//! tool execution, and event emission.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

mod dynamic_tools;

use proteus_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    contracts::{CompactionInput, ToolExposureRequest},
    domain::{
        AgentOutput, ContextBundle, Event, HistoryCompactionReport, TokenUsageCategory,
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
const CODEX_SYSTEM_INSTRUCTIONS: &str = "\
You are running inside a Codex-shaped coding workflow. Act as a practical coding \
agent: inspect before editing, make narrow changes, use tools only when they are \
necessary, and keep the user's current goal in focus. For non-trivial work, make \
a short working plan in the assistant turn before using tools. If files are \
changed, prefer targeted verification with the available tools when a relevant \
command is clear. Do not invent dates or times; omit them unless the user \
supplied them or you verified them with a tool.";
const CODEX_EXECUTE_DEVELOPER_INSTRUCTIONS: &str = "\
Codex execute phase: continue using tools until the task is actually handled or \
blocked. Prefer reading the relevant files before patching. Keep tool use \
focused, avoid unrelated refactors, and do not call remember_fact for temporary \
task notes. If dynamic tool exposure hides a needed tool, use the Proteus \
meta-tools to discover and call it. Once no more tools are needed, provide a \
concise draft answer that includes what changed, what was verified, and any \
remaining gap.";
const CODEX_FINAL_DEVELOPER_INSTRUCTIONS: &str = "\
Codex final phase: produce the final user-facing answer. Do not call tools. \
Summarize the completed work, mention verification that actually ran, and call \
out any remaining gap plainly. Keep it concise.";

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

        match run_codex_loop(input, host) {
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
    let mut tool_round_limit_reached = true;
    let mut draft_finish_reason = None;
    let mut executed_tools = Vec::new();

    for _round in 0..MAX_TOOL_ROUNDS {
        let prepared = request_from_state(
            &input,
            host,
            &model_messages,
            CODEX_SYSTEM_INSTRUCTIONS,
            Some(CODEX_EXECUTE_DEVELOPER_INSTRUCTIONS),
            "codex_execute",
        )?;
        if let Some(report) = prepared.compaction.as_ref() {
            compactions.push(report.clone());
            if report.changed {
                current_turn_messages_start = replace_after_compaction(
                    &prepared.request.messages,
                    &mut model_messages,
                    &mut persistent_messages,
                    current_user_message_id,
                    &["draft"],
                )?;
            }
        }
        let request = prepared.request;
        emit_event(
            host,
            &Event::ModelRequestPrepared {
                model: request.model.clone(),
            },
        )?;
        let response = complete_model(host, &request, "codex_execute")?;
        emit_event(
            host,
            &Event::ModelResponseReceived {
                finish_reason: response.finish_reason.clone(),
            },
        )?;

        let finish_reason = response.finish_reason.clone();
        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        let phase = if should_run_tools {
            "execute"
        } else if tool_rounds > 0 {
            "draft"
        } else {
            "final"
        };
        let assistant_message =
            with_workflow_phase(response.message.clone(), CODEX_LOOP_MODULE_ID, phase);
        model_messages.push(assistant_message.clone());

        if should_run_tools {
            persistent_messages.push(assistant_message);
            tool_rounds += 1;
            for call in response.tool_calls {
                executed_tools.push(call.name.clone());
                let result = execute_or_handle_tool(host, &input, &call, "codex_execute")?;
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

        if tool_rounds == 0 {
            persistent_messages.push(assistant_message.clone());
            let output = AgentOutput::new(
                output_text(
                    &assistant_message,
                    &model_messages[current_turn_messages_start..],
                ),
                output_metadata_with_extra(
                    CODEX_LOOP_MODULE_ID,
                    &input,
                    &model_messages,
                    context_chunks,
                    context_token_estimate,
                    json!({
                        "max_tool_rounds": MAX_TOOL_ROUNDS,
                        "tool_rounds": tool_rounds,
                        "tool_round_limit_reached": false,
                        "phases": ["execute"],
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
            let new_messages_start =
                current_turn_start(&persistent_messages, current_user_message_id);
            return Ok(PluginWorkflowOutput {
                output,
                messages: persistent_messages,
                new_messages_start: Some(new_messages_start),
                compactions,
            });
        }

        draft_finish_reason = Some(finish_reason);
        tool_round_limit_reached = false;
        break;
    }

    let prepared = request_from_state_with_options(
        &input,
        host,
        &model_messages,
        CODEX_SYSTEM_INSTRUCTIONS,
        Some(CODEX_FINAL_DEVELOPER_INSTRUCTIONS),
        "codex_final",
        RequestOptions {
            expose_tools: false,
            include_dynamic_meta_tools: false,
        },
    )?;
    if let Some(report) = prepared.compaction.as_ref() {
        compactions.push(report.clone());
        if report.changed {
            current_turn_messages_start = replace_after_compaction(
                &prepared.request.messages,
                &mut model_messages,
                &mut persistent_messages,
                current_user_message_id,
                &["draft"],
            )?;
        }
    }
    let final_request = prepared.request.with_tool_choice(ToolChoice::None);
    emit_event(
        host,
        &Event::ModelRequestPrepared {
            model: final_request.model.clone(),
        },
    )?;
    let final_response = complete_model(host, &final_request, "codex_final")?;
    emit_event(
        host,
        &Event::ModelResponseReceived {
            finish_reason: final_response.finish_reason.clone(),
        },
    )?;

    let final_message = with_workflow_phase(final_response.message, CODEX_LOOP_MODULE_ID, "final");
    model_messages.push(final_message.clone());
    persistent_messages.push(final_message.clone());
    let output = AgentOutput::new(
        output_text(
            &final_message,
            &model_messages[current_turn_messages_start..],
        ),
        output_metadata_with_extra(
            CODEX_LOOP_MODULE_ID,
            &input,
            &model_messages,
            context_chunks,
            context_token_estimate,
            json!({
                "max_tool_rounds": MAX_TOOL_ROUNDS,
                "tool_rounds": tool_rounds,
                "tool_round_limit_reached": tool_round_limit_reached,
                "draft_finish_reason": draft_finish_reason,
                "phases": ["execute", "final"],
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
    Ok(PluginWorkflowOutput {
        output,
        messages: persistent_messages,
        new_messages_start: Some(new_messages_start),
        compactions,
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
    request_from_state_with_options(
        input,
        host,
        messages,
        system_instructions,
        developer_instructions,
        phase,
        RequestOptions {
            expose_tools: true,
            include_dynamic_meta_tools: phase != "review",
        },
    )
}

fn request_from_state_with_options(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
    system_instructions: &str,
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
            .with_reasoning(input.runtime.reasoning.clone());
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
    match &mut request.metadata {
        Value::Object(metadata) => {
            metadata.insert(key.to_owned(), json!(value));
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

    let plan_workflow: WorkflowObject =
        PluginWorkflow_TO::from_value(CodingPlanExecuteReviewWorkflow, TD_Opaque);
    registry.register_workflow(RString::from(PLAN_EXECUTE_REVIEW_MODULE_ID), plan_workflow)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("coding-workflow"),
        description: RStr::from_str(
            "Workflow plugin providing coding.single_loop, coding.codex_loop, and coding.plan_execute_review through the workflow host API",
        ),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    use proteus_contracts::{
        abi_stable::sabi_trait::TD_Opaque,
        domain::{
            AgentTask, ContextChunk, ModelRef, ReasoningConfig, new_call_id, new_session_id,
            new_thread_id, new_turn_id,
        },
        plugin::{
            PluginWorkflowHost, PluginWorkflowHost_TO, PluginWorkflowHostError,
            PluginWorkflowRuntimeInfo,
        },
    };

    #[test]
    fn insert_request_metadata_u32_preserves_existing_object_fields() {
        let mut request = CanonicalModelRequest::new(ModelRef::new("fake", "model"), Vec::new())
            .with_metadata(json!({ "existing": true }));

        insert_request_metadata_u32(&mut request, "compaction_trigger_tokens", 12_800);

        assert_eq!(request.metadata["existing"], true);
        assert_eq!(request.metadata["compaction_trigger_tokens"], 12_800);
    }

    #[test]
    fn insert_request_metadata_u32_wraps_non_object_metadata() {
        let mut request = CanonicalModelRequest::new(ModelRef::new("fake", "model"), Vec::new())
            .with_metadata(json!("legacy"));

        insert_request_metadata_u32(&mut request, "compaction_trigger_tokens", 12_800);

        assert_eq!(request.metadata["compaction_trigger_tokens"], 12_800);
        assert_eq!(request.metadata["previous_metadata"], "legacy");
    }

    #[test]
    fn token_usage_snapshot_reads_compaction_trigger_metadata() {
        let mut request = CanonicalModelRequest::new(ModelRef::new("fake", "model"), Vec::new())
            .with_metadata(json!({ "compaction_trigger_tokens": 12_800 }));
        request.limits.max_input_tokens = Some(16_000);

        let usage = request_token_usage_snapshot(&request, None, "execute");

        assert_eq!(usage.max_input_tokens, Some(16_000));
        assert_eq!(usage.compaction_trigger_tokens, Some(12_800));
    }

    #[test]
    fn empty_text_response_gets_placeholder() {
        let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

        assert_eq!(message_text(&message), "<empty model response>");
    }

    #[test]
    fn empty_final_output_falls_back_to_latest_tool_result() {
        let result = ToolResult::new(
            proteus_contracts::domain::new_call_id(),
            false,
            "usage: skatewind --place NAME".to_owned(),
            Vec::new(),
            Some("process exited with code 1".to_owned()),
            json!({}),
        );
        let messages = vec![CanonicalMessage::new(
            MessageRole::Tool,
            vec![ContentPart::ToolResult { result }],
        )];
        let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

        let text = output_text(&message, &messages);

        assert!(text.contains("Model returned an empty final response"));
        assert!(text.contains("usage: skatewind --place NAME"));
        assert!(text.contains("process exited with code 1"));
    }

    #[test]
    fn empty_final_output_does_not_fall_back_to_previous_turn_tool_result() {
        let result = ToolResult::new(
            proteus_contracts::domain::new_call_id(),
            false,
            "old turn output".to_owned(),
            Vec::new(),
            Some("old turn error".to_owned()),
            json!({}),
        );
        let history = [CanonicalMessage::new(
            MessageRole::Tool,
            vec![ContentPart::ToolResult { result }],
        )];
        let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

        let text = output_text(&message, &history[history.len()..]);

        assert_eq!(text, "<empty model response>");
    }

    #[test]
    fn estimates_tokens_from_text_context_and_tool_results() {
        let result = ToolResult::ok(proteus_contracts::domain::new_call_id(), "abcd")
            .with_metadata(json!({}));
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
        visible_tools: Mutex<Vec<ToolSpec>>,
        selected_tools: Mutex<Vec<ToolSpec>>,
        executed_calls: Mutex<Vec<ToolCall>>,
        compactions: Mutex<Vec<CompactionInput>>,
        compaction_outputs: Mutex<VecDeque<proteus_contracts::contracts::CompactionOutput>>,
    }

    impl FakeHost {
        fn with_responses(responses: Vec<CanonicalModelResponse>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(VecDeque::from(responses)),
                visible_tools: Mutex::new(Vec::new()),
                selected_tools: Mutex::new(Vec::new()),
                executed_calls: Mutex::new(Vec::new()),
                compactions: Mutex::new(Vec::new()),
                compaction_outputs: Mutex::new(VecDeque::new()),
            }
        }

        fn with_tools(
            mut self,
            visible_tools: Vec<ToolSpec>,
            selected_tools: Vec<ToolSpec>,
        ) -> Self {
            self.visible_tools = Mutex::new(visible_tools);
            self.selected_tools = Mutex::new(selected_tools);
            self
        }

        fn with_compaction_outputs(
            mut self,
            outputs: Vec<proteus_contracts::contracts::CompactionOutput>,
        ) -> Self {
            self.compaction_outputs = Mutex::new(VecDeque::from(outputs));
            self
        }
    }

    impl PluginWorkflowHost for FakeHost {
        fn is_cancelled(&self) -> RResult<bool, PluginWorkflowHostError> {
            RResult::ROk(false)
        }

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
            self.compactions
                .lock()
                .expect("compactions")
                .push(input.clone());
            let output = self
                .compaction_outputs
                .lock()
                .expect("compaction outputs")
                .pop_front()
                .unwrap_or_else(|| {
                    proteus_contracts::contracts::CompactionOutput::unchanged(input.messages)
                });
            RResult::ROk(RString::from(
                serde_json::to_string(&output).expect("compaction output json"),
            ))
        }

        fn visible_tools_json(&self, _cwd: RString) -> RResult<RString, PluginWorkflowHostError> {
            RResult::ROk(RString::from(
                serde_json::to_string(&*self.visible_tools.lock().expect("visible tools"))
                    .expect("visible tools json"),
            ))
        }

        fn select_tools_json(
            &self,
            _request_json: RString,
        ) -> RResult<RString, PluginWorkflowHostError> {
            let output = proteus_contracts::contracts::ToolExposureOutput::new(
                self.selected_tools.lock().expect("selected tools").clone(),
            );
            RResult::ROk(RString::from(
                serde_json::to_string(&output).expect("tool exposure output json"),
            ))
        }

        fn execute_tool_json(
            &self,
            _task_json: RString,
            call_json: RString,
        ) -> RResult<RString, PluginWorkflowHostError> {
            let call: ToolCall = serde_json::from_str(call_json.as_str()).expect("tool call json");
            self.executed_calls
                .lock()
                .expect("executed calls")
                .push(call.clone());
            let result = ToolResult::ok(call.id.clone(), format!("{} ok", call.name))
                .with_metadata(json!({ "inner": true }));
            RResult::ROk(RString::from(
                serde_json::to_string(&result).expect("tool result json"),
            ))
        }

        fn emit_event_json(&self, event_json: RString) -> RResult<(), PluginWorkflowHostError> {
            let event: Event = serde_json::from_str(event_json.as_str()).expect("event json");
            self.events.lock().expect("events").push(event);
            RResult::ROk(())
        }
    }

    fn workflow_input(text: &str) -> PluginWorkflowInput {
        PluginWorkflowInput {
            task: AgentTask::new(text, std::env::current_dir().expect("cwd")),
            history: Vec::new(),
            runtime: PluginWorkflowRuntimeInfo {
                session_id: new_session_id(),
                thread_id: new_thread_id(),
                turn_id: new_turn_id(),
                model_ref: ModelRef::new("fake", "model"),
                reasoning: ReasoningConfig::default(),
                max_input_tokens: Some(16_000),
                model_timeout_ms: 120_000,
                context_timeout_ms: 30_000,
            },
        }
    }

    fn test_tool(name: &str, description: &str, safety: ToolSafety) -> ToolSpec {
        ToolSpec::new(
            name,
            description,
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace path" }
                },
                "required": ["path"]
            }),
            safety,
        )
    }

    fn tool_call_response(call: ToolCall) -> CanonicalModelResponse {
        CanonicalModelResponse::new(
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            ),
            vec![call],
            FinishReason::ToolCalls,
        )
    }

    #[test]
    fn codex_loop_runs_tool_round_then_finalizes_without_tools() {
        let input = workflow_input("change code");
        let input_json = serde_json::to_string(&input).expect("input json");
        let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
        let apply_patch = test_tool("apply_patch", "Apply patch", ToolSafety::WritesFiles);
        let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
        let mut host = FakeHost::with_responses(vec![
            tool_call_response(call.clone()),
            CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, "draft after tools"),
                Vec::new(),
                FinishReason::Stop,
            ),
            CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, "final answer"),
                Vec::new(),
                FinishReason::Stop,
            ),
        ])
        .with_tools(vec![read_file.clone(), apply_patch], vec![read_file]);
        let mut host_to: PluginWorkflowHostMut<'_> =
            PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

        let output_json =
            match CodingCodexLoopWorkflow.run_json(RString::from(input_json), &mut host_to) {
                RResult::ROk(json) => json,
                RResult::RErr(error) => panic!("workflow failed: {}", error.message),
            };
        let output: PluginWorkflowOutput =
            serde_json::from_str(output_json.as_str()).expect("output json");
        drop(host_to);

        assert_eq!(output.output.text, "final answer");
        assert_eq!(
            output.output.metadata["workflow"]["module_id"],
            CODEX_LOOP_MODULE_ID
        );
        assert_eq!(
            output.output.metadata["phases"],
            json!(["execute", "final"])
        );
        assert_eq!(output.output.metadata["tool_rounds"], json!(1));
        assert_eq!(
            output.output.metadata["tool_round_limit_reached"],
            json!(false)
        );
        assert_eq!(output.new_messages_start, Some(0));

        let persisted = output
            .messages
            .iter()
            .map(|message| (message.role.clone(), message_text(message)))
            .collect::<Vec<_>>();
        assert_eq!(
            persisted,
            vec![
                (MessageRole::User, "change code".to_owned()),
                (MessageRole::Assistant, "<empty model response>".to_owned()),
                (MessageRole::Tool, "<empty model response>".to_owned()),
                (MessageRole::Assistant, "final answer".to_owned()),
            ]
        );
        let persisted_tool_output = output
            .messages
            .iter()
            .find_map(|message| {
                message.parts.iter().find_map(|part| match part {
                    ContentPart::ToolResult { result } => Some(result.output.as_str()),
                    _ => None,
                })
            })
            .expect("persisted tool result");
        assert_eq!(persisted_tool_output, "read_file ok");
        assert!(
            output
                .messages
                .iter()
                .all(|message| message_text(message) != "draft after tools")
        );

        let executed_calls = host.executed_calls.lock().expect("executed calls");
        assert_eq!(executed_calls.len(), 1);
        assert_eq!(executed_calls[0].name, "read_file");

        let requests = host.requests.lock().expect("requests");
        assert_eq!(requests.len(), 3);
        assert!(
            requests[0]
                .instructions
                .iter()
                .any(|instruction| instruction.text.contains("Codex execute phase"))
        );
        assert!(
            requests[0]
                .tools
                .iter()
                .any(|tool| tool.name == dynamic_tools::TOOL_CALL)
        );
        assert!(requests[1].messages.iter().any(|message| {
            message.parts.iter().any(|part| match part {
                ContentPart::ToolResult { result } => result.output == "read_file ok",
                _ => false,
            })
        }));
        assert_eq!(requests[2].tool_choice, ToolChoice::None);
        assert!(requests[2].tools.is_empty());
        assert!(
            !requests[2]
                .instructions
                .iter()
                .any(|instruction| instruction.text.contains("full tool catalog"))
        );

        let compactions = host.compactions.lock().expect("compactions");
        assert_eq!(compactions.len(), 3);
        assert_eq!(compactions[0].reason.as_deref(), Some("codex_execute"));
        assert_eq!(compactions[1].reason.as_deref(), Some("codex_execute"));
        assert_eq!(compactions[2].reason.as_deref(), Some("codex_final"));
    }

    #[test]
    fn codex_loop_errors_when_changed_compaction_drops_current_user_message() {
        let input = workflow_input("change code");
        let input_json = serde_json::to_string(&input).expect("input json");
        let mut bad_output = proteus_contracts::contracts::CompactionOutput::changed(
            vec![CanonicalMessage::text(MessageRole::User, "summary only")],
            Some("summary only".to_owned()),
        );
        bad_output.metadata = json!({
            "input_messages": 2,
            "output_messages": 1,
            "original_token_estimate": 100,
            "output_token_estimate": 10,
        });
        let mut host = FakeHost::default().with_compaction_outputs(vec![bad_output]);
        let mut host_to: PluginWorkflowHostMut<'_> =
            PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

        let error = match CodingCodexLoopWorkflow.run_json(RString::from(input_json), &mut host_to)
        {
            RResult::ROk(_) => panic!("workflow unexpectedly succeeded"),
            RResult::RErr(error) => error,
        };
        drop(host_to);

        assert!(
            error
                .message
                .as_str()
                .contains("dropped the current user message")
        );
        assert!(host.requests.lock().expect("requests").is_empty());
        assert!(
            host.events
                .lock()
                .expect("events")
                .iter()
                .all(|event| !matches!(event, Event::TurnFinished { .. }))
        );
    }

    #[test]
    fn single_loop_calls_host_and_returns_persistent_messages() {
        let input = workflow_input("hello");
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
        // Потолок окна из runtime должен оказаться в лимитах запроса —
        // иначе снимок TokenUsageUpdated уедет без max_input_tokens.
        assert_eq!(requests[0].limits.max_input_tokens, Some(16_000));
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
        // Снимок несёт потолок окна — это знаменатель для бублика контекста в web UI.
        assert!(events.iter().any(|event| matches!(
            event,
            Event::TokenUsageUpdated { usage } if usage.max_input_tokens == Some(16_000)
        )));
        // No-op compactor output does not declare an autocompact trigger, so
        // the UI must not show a fake threshold marker.
        assert!(events.iter().any(|event| matches!(
            event,
            Event::TokenUsageUpdated { usage } if usage.compaction_trigger_tokens.is_none()
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::TurnFinished { .. }))
        );
    }

    #[test]
    fn single_loop_adds_dynamic_meta_tools_when_tool_exposure_hides_candidates() {
        let input = workflow_input("inspect history");
        let input_json = serde_json::to_string(&input).expect("input json");
        let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
        let git_log = test_tool("git_log", "Show commit history", ToolSafety::ReadOnly);
        let mut host =
            FakeHost::default().with_tools(vec![read_file.clone(), git_log], vec![read_file]);
        let mut host_to: PluginWorkflowHostMut<'_> =
            PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);

        let output_json = match CodingSingleLoopWorkflow::default()
            .run_json(RString::from(input_json), &mut host_to)
        {
            RResult::ROk(json) => json,
            RResult::RErr(error) => panic!("workflow failed: {}", error.message),
        };
        let _output: PluginWorkflowOutput =
            serde_json::from_str(output_json.as_str()).expect("output json");
        drop(host_to);

        let requests = host.requests.lock().expect("requests");
        let tool_names = requests[0]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            tool_names,
            vec![
                "read_file",
                dynamic_tools::TOOL_SEARCH,
                dynamic_tools::TOOL_DESCRIBE,
                dynamic_tools::TOOL_CALL,
            ]
        );
        assert!(
            requests[0]
                .instructions
                .iter()
                .any(|instruction| instruction.text.contains("full tool catalog"))
        );
    }

    #[test]
    fn proteus_tool_describe_returns_policy_visible_hidden_schema() {
        let input = workflow_input("describe hidden tool");
        let git_log = test_tool("git_log", "Show commit history", ToolSafety::ReadOnly);
        let mut host = FakeHost::default().with_tools(vec![git_log], Vec::new());
        let mut host_to: PluginWorkflowHostMut<'_> =
            PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
        let call = ToolCall::new(
            new_call_id(),
            dynamic_tools::TOOL_DESCRIBE,
            json!({ "name": "git_log" }),
        );

        let result =
            dynamic_tools::handle_meta_tool_call(&mut host_to, &input, &call, "execute").unwrap();
        drop(host_to);
        let output: Value = serde_json::from_str(&result.output).expect("describe output json");

        assert!(result.ok);
        assert_eq!(result.call_id, call.id);
        assert_eq!(output["name"], "git_log");
        assert_eq!(output["required_args"], Value::Null);
        assert_eq!(output["input_schema"]["required"], json!(["path"]));
    }

    #[test]
    fn proteus_tool_search_returns_compact_policy_visible_matches() {
        let input = workflow_input("search hidden tools");
        let git_log = test_tool("git_log", "Show commit history", ToolSafety::ReadOnly);
        let shell = test_tool("shell", "Run terminal commands", ToolSafety::RunsCommands);
        let mut host = FakeHost::default().with_tools(vec![git_log, shell], Vec::new());
        let mut host_to: PluginWorkflowHostMut<'_> =
            PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
        let call = ToolCall::new(
            new_call_id(),
            dynamic_tools::TOOL_SEARCH,
            json!({ "query": "commit history", "limit": 3 }),
        );

        let result =
            dynamic_tools::handle_meta_tool_call(&mut host_to, &input, &call, "execute").unwrap();
        drop(host_to);
        let output: Value = serde_json::from_str(&result.output).expect("search output json");

        assert!(result.ok);
        assert_eq!(result.call_id, call.id);
        assert_eq!(output["matches"][0]["name"], "git_log");
        assert_eq!(output["matches"][0]["input_schema"], Value::Null);
        assert_eq!(output["matches"][0]["required_args"], json!(["path"]));
    }

    #[test]
    fn proteus_tool_call_executes_hidden_tool_and_remaps_result_to_outer_call_id() {
        let outer_call = ToolCall::new(
            new_call_id(),
            dynamic_tools::TOOL_CALL,
            json!({
                "name": "hidden_echo",
                "args": { "path": "README.md" }
            }),
        );
        let input = workflow_input("call hidden tool");
        let input_json = serde_json::to_string(&input).expect("input json");
        let hidden_echo = test_tool("hidden_echo", "Echo hidden file", ToolSafety::ReadOnly);
        let mut host = FakeHost::with_responses(vec![
            tool_call_response(outer_call.clone()),
            CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, "final"),
                Vec::new(),
                FinishReason::Stop,
            ),
        ])
        .with_tools(vec![hidden_echo], Vec::new());
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

        let executed_calls = host.executed_calls.lock().expect("executed calls");
        assert_eq!(executed_calls.len(), 1);
        assert_eq!(executed_calls[0].name, "hidden_echo");
        assert_ne!(executed_calls[0].id, outer_call.id);

        let result = output
            .messages
            .iter()
            .find_map(|message| {
                message.parts.iter().find_map(|part| match part {
                    ContentPart::ToolResult { result } => Some(result),
                    _ => None,
                })
            })
            .expect("tool result");
        assert_eq!(result.call_id, outer_call.id);
        assert_eq!(
            result.metadata["deferred_tool"]["name"],
            Value::String("hidden_echo".to_owned())
        );
        assert_eq!(
            result.metadata["deferred_tool"]["inner_call_id"],
            Value::String(executed_calls[0].id.clone())
        );
    }

    #[test]
    fn proteus_tool_call_rejects_meta_tool_recursion_without_execution() {
        let input = workflow_input("bad recursive call");
        let mut host = FakeHost::default();
        let mut host_to: PluginWorkflowHostMut<'_> =
            PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
        let call = ToolCall::new(
            new_call_id(),
            dynamic_tools::TOOL_CALL,
            json!({
                "name": dynamic_tools::TOOL_SEARCH,
                "args": { "query": "anything" }
            }),
        );

        let result =
            dynamic_tools::handle_meta_tool_call(&mut host_to, &input, &call, "execute").unwrap();
        drop(host_to);

        assert!(!result.ok);
        assert_eq!(result.call_id, call.id);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("cannot call Proteus meta-tools")
        );
        assert!(
            host.executed_calls
                .lock()
                .expect("executed calls")
                .is_empty()
        );
    }

    #[test]
    fn proteus_tool_call_rejects_non_readonly_hidden_tool_in_plan_phase() {
        let input = workflow_input("plan write");
        let write_file = test_tool("write_file", "Write a file", ToolSafety::WritesFiles);
        let mut host = FakeHost::default().with_tools(vec![write_file], Vec::new());
        let mut host_to: PluginWorkflowHostMut<'_> =
            PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
        let call = ToolCall::new(
            new_call_id(),
            dynamic_tools::TOOL_CALL,
            json!({
                "name": "write_file",
                "args": { "path": "README.md" }
            }),
        );

        let result =
            dynamic_tools::handle_meta_tool_call(&mut host_to, &input, &call, "plan").unwrap();
        drop(host_to);

        assert!(!result.ok);
        assert_eq!(result.call_id, call.id);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("plan phase")
        );
        assert!(
            host.executed_calls
                .lock()
                .expect("executed calls")
                .is_empty()
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
                reasoning: ReasoningConfig::new(Some("high".to_owned()), true),
                max_input_tokens: Some(16_000),
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
        let persisted = output
            .messages
            .iter()
            .map(|message| (message.role.clone(), message_text(message)))
            .collect::<Vec<_>>();
        assert_eq!(
            persisted,
            vec![
                (MessageRole::User, "change code".to_owned()),
                (MessageRole::Assistant, "final".to_owned()),
            ]
        );
        assert!(
            output
                .messages
                .iter()
                .all(|message| message.metadata["workflow_phase"] != "plan")
        );

        let requests = host.requests.lock().expect("requests");
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].tool_choice, ToolChoice::Auto);
        assert_eq!(
            requests[0].reasoning,
            ReasoningConfig::new(Some("high".to_owned()), true)
        );
        assert!(
            requests[0]
                .tools
                .iter()
                .all(|tool| matches!(tool.safety, ToolSafety::ReadOnly))
        );
        assert_eq!(requests[2].tool_choice, ToolChoice::None);
        assert_eq!(requests[2].tools.len(), 0);

        let compactions = host.compactions.lock().expect("compactions");
        assert_eq!(compactions.len(), 3);
        assert_eq!(compactions[2].reason.as_deref(), Some("review"));
        assert_eq!(compactions[2].max_tokens, Some(12_800));
        assert!(
            compactions[2]
                .messages
                .iter()
                .any(|message| message_text(message) == "draft")
        );
    }
}
