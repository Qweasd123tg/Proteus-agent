use proteus_contracts::{
    contracts::WorkflowFailure,
    domain::Event,
    model_standard::FinishReason,
    process_module::{
        ProcessModuleError, WorkflowModuleHostMut, WorkflowModuleInput, WorkflowModuleOutput,
    },
};
use serde_json::{Value, json};

use crate::{
    codex_sampling::{StreamRetryConfig, complete_sampling_request},
    codex_tools::CodexToolRun,
    host::{emit_event, request_from_state_with_instruction_blocks},
    metadata::output_metadata_with_extra,
    output_text::message_text,
    scaffold::{PersistentRepair, TurnScaffold},
    token_accounting::LastModelUsage,
    validation::{response_output_message, validate_codex_model_response},
};

pub(crate) fn run_codex_loop(
    input: WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
    module_id: &str,
) -> Result<WorkflowModuleOutput, WorkflowFailure> {
    let stream_retry = StreamRetryConfig::from_config(&input.config)?;
    let mut turn = TurnScaffold::begin(host, &input).map_err(WorkflowFailure::from)?;
    super::codex_recovery::normalize_missing_tool_outputs(&mut turn.model_messages);
    match run_loop(&input, host, module_id, &mut turn, stream_retry) {
        Ok((text, metadata)) => turn
            .finish(host, text, metadata)
            .map_err(|error| failure_with_history(error, &turn)),
        Err(error) => Err(failure_with_history(error, &turn)),
    }
}

fn failure_with_history(error: ProcessModuleError, turn: &TurnScaffold) -> WorkflowFailure {
    let mut failure = WorkflowFailure::from(error);
    match turn.history_update() {
        Ok(Some(history)) => failure.with_history(history),
        Ok(None) => failure,
        Err(history_error) => {
            failure.message = format!(
                "{}; failed to collect workflow history: {}",
                failure.message, history_error.message
            );
            failure
        }
    }
}

fn run_loop(
    input: &WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
    module_id: &str,
    turn: &mut TurnScaffold,
    stream_retry: StreamRetryConfig,
) -> Result<(String, Value), ProcessModuleError> {
    let mut tools = CodexToolRun::default();
    let mut last_usage: Option<LastModelUsage> = None;

    loop {
        let prepared = request_from_state_with_instruction_blocks(
            input,
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
            turn.checkpoint(host, &[])?;
        }
        let mut request = prepared.request;
        let response =
            complete_sampling_request(host, input, turn, &mut request, stream_retry, &mut tools)?;
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
        let assistant_message = response_output_message("codex_loop", &response)?.clone();
        if let Some(usage) = response.usage.clone() {
            last_usage = Some(LastModelUsage {
                usage,
                message_count: turn
                    .model_messages
                    .len()
                    .saturating_sub(response.tool_calls.len()),
            });
        }

        if should_run_tools {
            turn.checkpoint(host, &[])?;
            continue;
        }
        turn.checkpoint(host, &[])?;
        if model_requests_follow_up {
            continue;
        }

        let text = message_text(&assistant_message);
        let metadata = output_metadata_with_extra(
            module_id,
            input,
            &turn.model_messages,
            turn.context_chunks,
            turn.context_token_estimate,
            json!({
                "tool_rounds": tools.tool_rounds,
                "phases": ["turn_loop"],
                "executed_tools": tools.executed_tools,
            }),
        );
        return Ok((text, metadata));
    }
}
