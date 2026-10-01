use crate::{MAX_TOOL_ROUNDS, PLAN_EXECUTE_REVIEW_MODULE_ID};
use crate::{
    host::{complete_model, emit_event, execute_or_handle_tool, request_from_state},
    metadata::{output_metadata_with_extra, with_workflow_phase},
    output_text::output_text,
    scaffold::{PersistentRepair, TurnScaffold},
    token_accounting::LastModelUsage,
    validation::{response_output_message, validate_model_response},
};
use proteus_contracts::{
    contracts::WorkflowFailure,
    domain::{Event, ToolChoice},
    model_standard::FinishReason,
    process_module::{
        ProcessModuleError, WorkflowModuleHostMut, WorkflowModuleInput, WorkflowModuleOutput,
    },
};
use serde_json::json;

/// Ограничение read-only tool loop в plan-фазе `coding.plan_execute_review`:
/// план имеет право посмотреть код, но не должен превращаться в execute.
const MAX_PLAN_TOOL_ROUNDS: usize = 3;
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

pub(crate) fn run_plan_execute_review(
    input: WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
) -> Result<WorkflowModuleOutput, WorkflowFailure> {
    crate::intents::reject(&input).map_err(WorkflowFailure::from)?;
    let mut turn = TurnScaffold::begin(host, &input).map_err(WorkflowFailure::from)?;
    run_loop(&input, host, &mut turn).map_err(|error| turn.failure(error))
}

fn run_loop(
    input: &WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
    turn: &mut TurnScaffold,
) -> Result<WorkflowModuleOutput, ProcessModuleError> {
    let mut last_usage: Option<LastModelUsage> = None;
    let mut plan_tool_rounds_used = 0usize;
    for plan_round in 0..=MAX_PLAN_TOOL_ROUNDS {
        let prepared = request_from_state(
            input,
            host,
            &turn.model_messages,
            PLAN_SYSTEM_INSTRUCTIONS,
            Some(PLAN_DEVELOPER_INSTRUCTIONS),
            "plan",
            last_usage.as_ref(),
        )?;
        if turn.apply_compaction_report(
            prepared.compaction.as_ref(),
            &prepared.request.messages,
            PersistentRepair::Rebuild,
        )? {
            last_usage = None;
            turn.checkpoint(host, &[])?;
        }
        let mut plan_request = prepared.request;
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
        let plan_messages = plan_response
            .messages
            .into_iter()
            .map(|message| with_workflow_phase(message, PLAN_EXECUTE_REVIEW_MODULE_ID, "plan"))
            .collect::<Vec<_>>();
        turn.model_messages.extend(plan_messages.iter().cloned());
        if let Some(usage) = plan_response.usage.clone() {
            last_usage = Some(LastModelUsage {
                usage,
                message_count: turn.model_messages.len(),
            });
        }

        let should_run_tools = plan_response.finish_reason == FinishReason::ToolCalls
            && !plan_response.tool_calls.is_empty();
        if forced_text_round || !should_run_tools {
            break;
        }
        plan_tool_rounds_used += 1;
        turn.persistent_messages.extend(plan_messages);
        turn.checkpoint_tools(host, input, &plan_response.tool_calls, "plan")?;
        for call in plan_response.tool_calls {
            let result = execute_or_handle_tool(host, input, &call, "plan")?;
            turn.append_tool_results(std::iter::once(result));
            turn.checkpoint(host, &[])?;
        }
    }

    let mut draft_finish_reason = None;
    let mut tool_round_limit_reached = true;
    for _round in 0..MAX_TOOL_ROUNDS {
        let prepared = request_from_state(
            input,
            host,
            &turn.model_messages,
            PLAN_SYSTEM_INSTRUCTIONS,
            Some(EXECUTE_DEVELOPER_INSTRUCTIONS),
            "execute",
            last_usage.as_ref(),
        )?;
        if turn.apply_compaction_report(
            prepared.compaction.as_ref(),
            &prepared.request.messages,
            PersistentRepair::Rebuild,
        )? {
            last_usage = None;
            turn.checkpoint(host, &[])?;
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
        validate_model_response("execute", &request, &response)?;

        let finish_reason = response.finish_reason.clone();
        turn.model_messages
            .extend(response.messages.iter().cloned());
        if let Some(usage) = response.usage.clone() {
            last_usage = Some(LastModelUsage {
                usage,
                message_count: turn.model_messages.len(),
            });
        }
        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        if should_run_tools {
            turn.persistent_messages
                .extend(response.messages.iter().cloned());
        }
        if !should_run_tools {
            draft_finish_reason = Some(finish_reason);
            tool_round_limit_reached = false;
            break;
        }

        turn.checkpoint_tools(host, input, &response.tool_calls, "execute")?;
        for call in response.tool_calls {
            let result = execute_or_handle_tool(host, input, &call, "execute")?;
            turn.append_tool_results(std::iter::once(result));
            turn.checkpoint(host, &[])?;
        }
    }

    let prepared = request_from_state(
        input,
        host,
        &turn.model_messages,
        PLAN_SYSTEM_INSTRUCTIONS,
        Some(REVIEW_DEVELOPER_INSTRUCTIONS),
        "review",
        last_usage.as_ref(),
    )?;
    if turn.apply_compaction_report(
        prepared.compaction.as_ref(),
        &prepared.request.messages,
        PersistentRepair::Rebuild,
    )? {
        turn.checkpoint(host, &[])?;
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
    validate_model_response("review", &review_request, &final_response)?;

    let final_message = response_output_message("review", &final_response)?.clone();
    turn.model_messages
        .extend(final_response.messages.iter().cloned());
    turn.persistent_messages
        .extend(final_response.messages.iter().cloned());
    turn.checkpoint(host, &[])?;
    let text = output_text(
        &final_message,
        &turn.model_messages[turn.current_turn_messages_start..],
    );
    let metadata = output_metadata_with_extra(
        PLAN_EXECUTE_REVIEW_MODULE_ID,
        input,
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
