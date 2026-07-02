//! Coding workflow plugin.
//!
//! Owns workflow control-flow, but every runtime capability goes through the
//! narrow workflow host API: context build, model completion, tool visibility,
//! tool execution, and event emission.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

mod dynamic_tools;
mod history;
mod host;
mod metadata;
mod output_text;
mod token_accounting;
mod validation;
mod workflows;

use proteus_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    domain::{AgentOutput, Event, ToolChoice, ToolSafety},
    model_standard::{CanonicalMessage, ContentPart, FinishReason, MessageRole},
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginWorkflow_TO,
        PluginWorkflowError, PluginWorkflowHostMut, PluginWorkflowInput, PluginWorkflowOutput,
        WorkflowObject,
    },
};
use serde_json::json;

#[cfg(test)]
pub(crate) use proteus_contracts::{
    contracts::CompactionInput,
    domain::{ContextBundle, TokenUsageSnapshot, TokenUsageSource, ToolCall, ToolResult, ToolSpec},
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, InstructionBlock, InstructionKind,
        TokenUsage,
    },
    plugin::PluginWorkflow,
};
use token_accounting::LastModelUsage;
#[cfg(test)]
use token_accounting::{estimate_message_tokens, request_token_usage_snapshot};

use history::{
    current_turn_start, persistent_messages_from_model_messages, replace_after_compaction,
};
use host::{
    build_context, complete_model, emit_event, execute_or_handle_tool, execute_tools,
    request_from_state, request_from_state_with_instruction_blocks,
};
#[cfg(test)]
use metadata::{insert_request_metadata_u32, prompt_cache_key};
use metadata::{output_metadata, output_metadata_with_extra, with_workflow_phase};
use output_text::{message_text, output_text};
use validation::validate_codex_model_response;
use workflows::EmptyFinalResponseMode;
pub use workflows::{
    CodingCodexLoopDiagnosticWorkflow, CodingCodexLoopWorkflow, CodingPlanExecuteReviewWorkflow,
    CodingSingleLoopWorkflow,
};

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

pub(crate) fn run_single_loop(
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
    let mut last_usage: Option<LastModelUsage> = None;

    for _round in 0..max_tool_rounds {
        let prepared = request_from_state(
            &input,
            host,
            &model_messages,
            SYSTEM_INSTRUCTIONS,
            None,
            "single_loop",
            last_usage.as_ref(),
        )?;
        if let Some(report) = prepared.compaction.as_ref() {
            compactions.push(report.clone());
            if report.changed {
                model_messages = prepared.request.messages.clone();
                persistent_messages = persistent_messages_from_model_messages(&model_messages);
                current_turn_messages_start =
                    current_turn_start(&model_messages, current_user_message_id);
                last_usage = None;
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
        if let Some(usage) = response.usage.clone() {
            last_usage = Some(LastModelUsage {
                usage,
                message_count: model_messages.len(),
            });
        }
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

        let results = execute_tools(host, &input, &response.tool_calls, "single_loop")?;
        for result in results {
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
        last_usage.as_ref(),
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

pub(crate) fn run_codex_loop(
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
    let mut last_usage: Option<LastModelUsage> = None;

    loop {
        let prepared = request_from_state_with_instruction_blocks(
            &input,
            host,
            &model_messages,
            input.runtime.instructions.clone(),
            None,
            "codex_loop",
            last_usage.as_ref(),
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
                last_usage = None;
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
        if let Some(usage) = response.usage.clone() {
            last_usage = Some(LastModelUsage {
                usage,
                message_count: model_messages.len(),
            });
        }

        if should_run_tools {
            tool_rounds += 1;
            for call in &response.tool_calls {
                executed_tools.push(call.name.clone());
            }
            let results = execute_tools(host, &input, &response.tool_calls, "codex_loop")?;
            for result in results {
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

pub(crate) fn run_plan_execute_review(
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
        None,
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
            None,
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
        None,
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
