//! Handle completed items while the model pump is still active. Tool results
//! are durable through checkpoints immediately, then drained into the prompt
//! after all model items, including on a failed sampling request.
use std::collections::HashSet;

use proteus_contracts::{
    contracts::WorkflowModelStreamItem,
    domain::{MessageId, ToolResult},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart,
    },
    process_module::{ProcessModuleError, WorkflowModuleHostMut, WorkflowModuleInput},
};

use crate::{codex_tools::CodexToolRun, scaffold::TurnScaffold};

pub(crate) fn sample(
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
    turn: &mut TurnScaffold,
    request: &CanonicalModelRequest,
    tools: &mut CodexToolRun,
) -> Result<CanonicalModelResponse, ProcessModuleError> {
    let mut sample = SampleProgress::default();
    let outcome = (|| {
        let cursor =
            host.start_model_stream_json(serde_json::to_string(request).map_err(json_error)?)?;
        loop {
            crate::host::ensure_not_cancelled(host)?;
            let item: WorkflowModelStreamItem =
                serde_json::from_str(&host.next_model_stream_json(cursor.clone())?)
                    .map_err(json_error)?;
            match item {
                WorkflowModelStreamItem::MessageCompleted { message } => {
                    sample.accept(
                        host,
                        input,
                        turn,
                        request,
                        tools,
                        std::slice::from_ref(&message),
                    )?;
                }
                WorkflowModelStreamItem::Response { response } => {
                    crate::validation::validate_codex_model_response(
                        "codex_loop",
                        request,
                        &response,
                    )?;
                    sample.accept(host, input, turn, request, tools, &response.messages)?;
                    return Ok(response);
                }
                WorkflowModelStreamItem::Error { failure } => {
                    sample.accept(
                        host,
                        input,
                        turn,
                        request,
                        tools,
                        &failure.completed_messages,
                    )?;
                    return Err(ProcessModuleError::from_model_failure(failure));
                }
            }
        }
    })();
    turn.append_tool_results(sample.results);
    outcome
}

#[derive(Default)]
struct SampleProgress {
    messages: HashSet<MessageId>,
    results: Vec<ToolResult>,
    had_tools: bool,
}

impl SampleProgress {
    fn accept(
        &mut self,
        host: &mut WorkflowModuleHostMut<'_>,
        input: &WorkflowModuleInput,
        turn: &mut TurnScaffold,
        request: &CanonicalModelRequest,
        tools: &mut CodexToolRun,
        messages: &[CanonicalMessage],
    ) -> Result<(), ProcessModuleError> {
        let mut calls = Vec::new();
        let mut changed = false;
        for message in messages {
            if !self.messages.insert(message.id) {
                continue;
            }
            changed = true;
            turn.model_messages.push(message.clone());
            turn.persistent_messages.push(message.clone());
            calls.extend(message.parts.iter().filter_map(|part| match &part.payload {
                ContentPart::ToolCall { call } => Some(call.clone()),
                _ => None,
            }));
        }
        if !calls.is_empty() {
            if !self.had_tools {
                tools.tool_rounds += 1;
                self.had_tools = true;
            }
            self.results
                .extend(tools.execute(host, input, turn, &calls, &request.tools)?);
        } else if changed {
            turn.checkpoint(host, &[])?;
        }
        Ok(())
    }
}

fn json_error(error: serde_json::Error) -> ProcessModuleError {
    ProcessModuleError::new(format!("invalid model stream payload: {error}"))
}
