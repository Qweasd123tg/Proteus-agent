//! Coding workflow reference process modules.
//!
//! Owns workflow control-flow, but every runtime capability goes through the
//! narrow workflow host API: context build, model completion, tool visibility,
//! tool execution, and event emission.

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
    domain::{Event, ToolChoice, ToolSafety},
    model_standard::FinishReason,
    process_module::{
        ModuleRegistry, ProcessModuleError, WorkflowModuleHostMut, WorkflowModuleInput,
        WorkflowModuleObject, WorkflowModuleOutput,
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
    process_module::WorkflowModule,
};
use token_accounting::LastModelUsage;
#[cfg(test)]
use token_accounting::{estimate_message_tokens, request_token_usage_snapshot};

use host::{
    complete_model, emit_event, execute_codex_tools, execute_or_handle_tool, execute_tools,
    request_from_state, request_from_state_with_instruction_blocks,
};
#[cfg(test)]
use metadata::{cache_routing_key, insert_request_metadata_u32};
use metadata::{output_metadata, output_metadata_with_extra, with_workflow_phase};
use output_text::{message_text, output_text};
use scaffold::{PersistentRepair, TurnScaffold};
use validation::{validate_codex_model_response, validate_model_response};
pub use workflows::{
    CodingCodexLoopWorkflow, CodingPlanExecuteReviewWorkflow, CodingSingleLoopWorkflow,
};

const SINGLE_LOOP_MODULE_ID: &str = "coding.single_loop";
const CODEX_LOOP_MODULE_ID: &str = "coding.codex_loop";
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
    input: WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
    max_tool_rounds: usize,
) -> Result<WorkflowModuleOutput, ProcessModuleError> {
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
    input: WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
    module_id: &str,
) -> Result<WorkflowModuleOutput, ProcessModuleError> {
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
        validate_codex_model_response("codex_loop", &request, &response)?;

        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        let model_requests_follow_up = response.end_turn == Some(false);
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
            let results = execute_codex_tools(
                host,
                &input,
                &response.tool_calls,
                &request.tools,
                "codex_loop",
            )?;
            turn.append_tool_results(results);
            continue;
        }
        if model_requests_follow_up {
            continue;
        }

        let text = message_text(&assistant_message);
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
    input: WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
) -> Result<WorkflowModuleOutput, ProcessModuleError> {
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

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let workflow: WorkflowModuleObject = Box::new(CodingSingleLoopWorkflow::default());
    if let Err(err) = registry.register_workflow(String::from(SINGLE_LOOP_MODULE_ID), workflow) {
        return Err(err);
    }

    let codex_workflow: WorkflowModuleObject = Box::new(CodingCodexLoopWorkflow);
    if let Err(err) = registry.register_workflow(String::from(CODEX_LOOP_MODULE_ID), codex_workflow)
    {
        return Err(err);
    }

    let plan_workflow: WorkflowModuleObject = Box::new(CodingPlanExecuteReviewWorkflow);
    registry.register_workflow(String::from(PLAN_EXECUTE_REVIEW_MODULE_ID), plan_workflow)
}

#[cfg(test)]
mod tests;
