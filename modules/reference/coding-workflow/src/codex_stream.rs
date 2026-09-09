//! Handle completed items while the model pump is still active. Tool results
//! are durable through checkpoints immediately, then drained into the prompt
//! after all model items, including on a failed sampling request.
use std::collections::HashSet;

use proteus_contracts::{
    contracts::WorkflowModelStreamItem,
    domain::MessageId,
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart,
    },
    process_module::{ProcessModuleError, WorkflowModuleHostMut, WorkflowModuleInput},
};

use crate::{codex_tools::CodexToolRun, scaffold::TurnScaffold};

mod in_flight;
use in_flight::InFlight;

pub(crate) fn sample(
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
    turn: &mut TurnScaffold,
    request: &CanonicalModelRequest,
    tools: &mut CodexToolRun,
) -> Result<CanonicalModelResponse, ProcessModuleError> {
    std::thread::scope(|scope| {
        let host = &*host;
        let mut sample = SampleProgress::default();
        let mut in_flight = InFlight::default();
        let mut accept = |messages: &[CanonicalMessage]| {
            sample.accept(host, turn, request, tools, messages, |batch, parallel| {
                in_flight.start(scope, host, input, batch, parallel)
            })
        };
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
                        accept(std::slice::from_ref(&message))?;
                    }
                    WorkflowModelStreamItem::Response { response } => {
                        crate::validation::validate_codex_model_response(
                            "codex_loop",
                            request,
                            &response,
                        )?;
                        accept(&response.messages)?;
                        return Ok(response);
                    }
                    WorkflowModelStreamItem::Error { failure } => {
                        accept(&failure.completed_messages)?;
                        return Err(ProcessModuleError::from_model_failure(failure));
                    }
                }
            }
        })();
        let (results, tool_failure) = in_flight.drain();
        turn.append_tool_results(results);
        match tool_failure {
            Some(error) => Err(error),
            None => outcome,
        }
    })
}

#[derive(Default)]
struct SampleProgress {
    messages: HashSet<MessageId>,
    had_tools: bool,
}

impl SampleProgress {
    fn accept(
        &mut self,
        host: &WorkflowModuleHostMut<'_>,
        turn: &mut TurnScaffold,
        request: &CanonicalModelRequest,
        tools: &mut CodexToolRun,
        messages: &[CanonicalMessage],
        mut start: impl FnMut(
            crate::codex_tools::CodexToolBatch,
            bool,
        ) -> Result<(), ProcessModuleError>,
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
            let batch = tools.prepare(host, turn, &calls, &request.tools)?;
            let parallel = batch.permits_parallel(&request.tools);
            start(batch, parallel)?;
        } else if changed {
            turn.checkpoint(host, &[])?;
        }
        Ok(())
    }
}

fn json_error(error: serde_json::Error) -> ProcessModuleError {
    ProcessModuleError::new(format!("invalid model stream payload: {error}"))
}
