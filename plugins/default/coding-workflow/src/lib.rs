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
mod scaffold;
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
    domain::{Event, ToolChoice, ToolSafety},
    model_standard::FinishReason,
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
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart,
        InstructionBlock, InstructionKind, MessageRole, TokenUsage,
    },
    plugin::PluginWorkflow,
};
use token_accounting::LastModelUsage;
#[cfg(test)]
use token_accounting::{estimate_message_tokens, request_token_usage_snapshot};

use host::{
    complete_model, emit_event, execute_or_handle_tool, execute_tools, request_from_state,
    request_from_state_with_instruction_blocks,
};
#[cfg(test)]
use metadata::{insert_request_metadata_u32, prompt_cache_key};
use metadata::{output_metadata, output_metadata_with_extra, with_workflow_phase};
use output_text::{message_text, output_text};
use scaffold::{PersistentRepair, TurnScaffold};
use validation::validate_model_response;
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
/// Ограничение read-only tool loop в plan-фазе `coding.plan_execute_review`:
/// план имеет право посмотреть код, но не должен превращаться в execute.
const MAX_PLAN_TOOL_ROUNDS: usize = 3;
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
    let mut turn = TurnScaffold::begin(host, &input)?;
    let mut last_usage: Option<LastModelUsage> = None;

    for _round in 0..max_tool_rounds {
        let prepared = request_from_state(
            &input,
            host,
            &turn.model_messages,
            SYSTEM_INSTRUCTIONS,
            None,
            "single_loop",
            last_usage.as_ref(),
        )?;
        if turn.apply_compaction_report(
            prepared.compaction.as_ref(),
            &prepared.request.messages,
            PersistentRepair::Rebuild,
        )? {
            last_usage = None;
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
        validate_model_response("single_loop", &request, &response)?;

        turn.model_messages.push(response.message.clone());
        turn.persistent_messages.push(response.message.clone());
        if let Some(usage) = response.usage.clone() {
            last_usage = Some(LastModelUsage {
                usage,
                message_count: turn.model_messages.len(),
            });
        }
        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        if !should_run_tools {
            let text = output_text(
                &response.message,
                &turn.model_messages[turn.current_turn_messages_start..],
            );
            let metadata = output_metadata(
                SINGLE_LOOP_MODULE_ID,
                &input,
                &turn.model_messages,
                turn.context_chunks,
                turn.context_token_estimate,
            );
            return turn.finish(host, text, metadata);
        }

        let results = execute_tools(host, &input, &response.tool_calls, "single_loop")?;
        turn.append_tool_results(results);
    }

    let prepared = request_from_state(
        &input,
        host,
        &turn.model_messages,
        SYSTEM_INSTRUCTIONS,
        None,
        "single_loop_final",
        last_usage.as_ref(),
    )?;
    turn.apply_compaction_report(
        prepared.compaction.as_ref(),
        &prepared.request.messages,
        PersistentRepair::Rebuild,
    )?;
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
    validate_model_response("single_loop_final", &request, &response)?;

    turn.model_messages.push(response.message.clone());
    turn.persistent_messages.push(response.message.clone());
    let text = output_text(
        &response.message,
        &turn.model_messages[turn.current_turn_messages_start..],
    );
    let metadata = output_metadata_with_extra(
        SINGLE_LOOP_MODULE_ID,
        &input,
        &turn.model_messages,
        turn.context_chunks,
        turn.context_token_estimate,
        json!({
            "max_tool_rounds": max_tool_rounds,
            "tool_round_limit_reached": true,
        }),
    );
    turn.finish(host, text, metadata)
}

pub(crate) fn run_codex_loop(
    input: PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    module_id: &str,
    empty_final_response_mode: EmptyFinalResponseMode,
) -> Result<PluginWorkflowOutput, PluginWorkflowError> {
    let mut turn = TurnScaffold::begin(host, &input)?;
    let mut tool_rounds = 0usize;
    let mut executed_tools = Vec::new();
    let mut last_usage: Option<LastModelUsage> = None;

    loop {
        let prepared = request_from_state_with_instruction_blocks(
            &input,
            host,
            &turn.model_messages,
            input.runtime.instructions.clone(),
            None,
            "codex_loop",
            last_usage.as_ref(),
        )?;
        if turn.apply_compaction_report(
            prepared.compaction.as_ref(),
            &prepared.request.messages,
            PersistentRepair::ReplaceAfter,
        )? {
            last_usage = None;
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
        validate_model_response("codex_loop", &request, &response)?;

        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        let assistant_message = response.message.clone();
        turn.model_messages.push(assistant_message.clone());
        turn.persistent_messages.push(assistant_message.clone());
        if let Some(usage) = response.usage.clone() {
            last_usage = Some(LastModelUsage {
                usage,
                message_count: turn.model_messages.len(),
            });
        }

        if should_run_tools {
            tool_rounds += 1;
            for call in &response.tool_calls {
                executed_tools.push(call.name.clone());
            }
            let results = execute_tools(host, &input, &response.tool_calls, "codex_loop")?;
            turn.append_tool_results(results);
            continue;
        }

        let text = match empty_final_response_mode {
            EmptyFinalResponseMode::Strict => message_text(&assistant_message),
            EmptyFinalResponseMode::LastToolResultDiagnostic => output_text(
                &assistant_message,
                &turn.model_messages[turn.current_turn_messages_start..],
            ),
        };
        let metadata = output_metadata_with_extra(
            module_id,
            &input,
            &turn.model_messages,
            turn.context_chunks,
            turn.context_token_estimate,
            json!({
                "tool_rounds": tool_rounds,
                "phases": ["turn_loop"],
                "executed_tools": executed_tools,
            }),
        );
        return turn.finish(host, text, metadata);
    }
}

pub(crate) fn run_plan_execute_review(
    input: PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
) -> Result<PluginWorkflowOutput, PluginWorkflowError> {
    let mut turn = TurnScaffold::begin(host, &input)?;

    let mut plan_tool_rounds_used = 0usize;
    for plan_round in 0..=MAX_PLAN_TOOL_ROUNDS {
        let prepared = request_from_state(
            &input,
            host,
            &turn.model_messages,
            PLAN_SYSTEM_INSTRUCTIONS,
            Some(PLAN_DEVELOPER_INSTRUCTIONS),
            "plan",
            None,
        )?;
        turn.apply_compaction_report(
            prepared.compaction.as_ref(),
            &prepared.request.messages,
            PersistentRepair::Rebuild,
        )?;
        let mut plan_request = prepared.request;
        plan_request
            .tools
            .retain(|tool| matches!(tool.safety, ToolSafety::ReadOnly));
        // Последняя итерация принудительно без tools: plan-фаза обязана
        // закончиться текстовым планом, а не подвисшим tool call.
        let forced_text_round = plan_round == MAX_PLAN_TOOL_ROUNDS;
        if forced_text_round {
            plan_request = plan_request.with_tool_choice(ToolChoice::None);
            plan_request.tools.clear();
        }
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
        validate_model_response("plan", &plan_request, &plan_response)?;
        let plan_message =
            with_workflow_phase(plan_response.message, PLAN_EXECUTE_REVIEW_MODULE_ID, "plan");
        turn.model_messages.push(plan_message.clone());

        let should_run_tools = plan_response.finish_reason == FinishReason::ToolCalls
            && !plan_response.tool_calls.is_empty();
        if forced_text_round || !should_run_tools {
            break;
        }
        plan_tool_rounds_used += 1;
        turn.persistent_messages.push(plan_message);
        for call in plan_response.tool_calls {
            let result = execute_or_handle_tool(host, &input, &call, "plan")?;
            turn.append_tool_results(std::iter::once(result));
        }
    }

    let mut draft_finish_reason = None;
    let mut tool_round_limit_reached = true;
    for _round in 0..MAX_TOOL_ROUNDS {
        let prepared = request_from_state(
            &input,
            host,
            &turn.model_messages,
            PLAN_SYSTEM_INSTRUCTIONS,
            Some(EXECUTE_DEVELOPER_INSTRUCTIONS),
            "execute",
            None,
        )?;
        turn.apply_compaction_report(
            prepared.compaction.as_ref(),
            &prepared.request.messages,
            PersistentRepair::Rebuild,
        )?;
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
        validate_model_response("execute", &request, &response)?;

        let finish_reason = response.finish_reason.clone();
        turn.model_messages.push(response.message.clone());
        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        if should_run_tools {
            turn.persistent_messages.push(response.message.clone());
        }
        if !should_run_tools {
            draft_finish_reason = Some(finish_reason);
            tool_round_limit_reached = false;
            break;
        }

        for call in response.tool_calls {
            let result = execute_or_handle_tool(host, &input, &call, "execute")?;
            turn.append_tool_results(std::iter::once(result));
        }
    }

    let prepared = request_from_state(
        &input,
        host,
        &turn.model_messages,
        PLAN_SYSTEM_INSTRUCTIONS,
        Some(REVIEW_DEVELOPER_INSTRUCTIONS),
        "review",
        None,
    )?;
    turn.apply_compaction_report(
        prepared.compaction.as_ref(),
        &prepared.request.messages,
        PersistentRepair::Rebuild,
    )?;
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
    validate_model_response("review", &review_request, &final_response)?;

    turn.model_messages.push(final_response.message.clone());
    turn.persistent_messages
        .push(final_response.message.clone());
    let text = output_text(
        &final_response.message,
        &turn.model_messages[turn.current_turn_messages_start..],
    );
    let metadata = output_metadata_with_extra(
        PLAN_EXECUTE_REVIEW_MODULE_ID,
        &input,
        &turn.model_messages,
        turn.context_chunks,
        turn.context_token_estimate,
        json!({
            "max_tool_rounds": MAX_TOOL_ROUNDS,
            "tool_round_limit_reached": tool_round_limit_reached,
            "draft_finish_reason": draft_finish_reason,
            "max_plan_tool_rounds": MAX_PLAN_TOOL_ROUNDS,
            "plan_tool_rounds_used": plan_tool_rounds_used,
            "phases": ["plan", "execute", "review"],
        }),
    );
    turn.finish(host, text, metadata)
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
