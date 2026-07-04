use proteus_contracts::{
    domain::{
        AgentOutput, CONTEXT_MESSAGE_NAME, Event, HistoryCompactionReport, MessageId, ToolResult,
    },
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
    plugin::{
        PluginWorkflowError, PluginWorkflowHostMut, PluginWorkflowInput, PluginWorkflowOutput,
    },
};
use serde_json::Value;

use crate::history::{
    current_turn_start, persistent_messages_from_model_messages, replace_after_compaction,
};
use crate::host::{build_context, emit_event};

pub(crate) enum PersistentRepair {
    Rebuild,
    ReplaceAfter,
}

pub(crate) struct TurnScaffold {
    pub(crate) model_messages: Vec<CanonicalMessage>,
    pub(crate) persistent_messages: Vec<CanonicalMessage>,
    pub(crate) current_user_message_id: MessageId,
    pub(crate) current_turn_messages_start: usize,
    pub(crate) context_chunks: usize,
    pub(crate) context_token_estimate: Option<u32>,
    compactions: Vec<HistoryCompactionReport>,
}

impl TurnScaffold {
    pub(crate) fn begin(
        host: &mut PluginWorkflowHostMut<'_>,
        input: &PluginWorkflowInput,
    ) -> Result<Self, PluginWorkflowError> {
        emit_event(
            host,
            &Event::TaskReceived {
                task: input.task.clone(),
            },
        )?;

        let bundle = build_context(host, input)?;
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
        let current_user_message_id = user_message.id;
        persistent_messages.push(user_message.clone());

        let mut model_messages = persistent_messages.clone();
        for chunk in bundle.chunks {
            model_messages.push(
                CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
                    .with_name(CONTEXT_MESSAGE_NAME),
            );
        }
        let current_turn_messages_start = model_messages.len();

        Ok(Self {
            model_messages,
            persistent_messages,
            current_user_message_id,
            current_turn_messages_start,
            context_chunks,
            context_token_estimate,
            compactions: Vec::new(),
        })
    }

    pub(crate) fn append_tool_results(&mut self, results: impl IntoIterator<Item = ToolResult>) {
        for result in results {
            let call_id = result.call_id.clone();
            let tool_result_message =
                CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                    .with_tool_call_id(call_id);
            self.model_messages.push(tool_result_message.clone());
            self.persistent_messages.push(tool_result_message);
        }
    }

    pub(crate) fn apply_compaction_report(
        &mut self,
        report: Option<&HistoryCompactionReport>,
        compacted_messages: &[CanonicalMessage],
        repair: PersistentRepair,
    ) -> Result<bool, PluginWorkflowError> {
        let Some(report) = report else {
            return Ok(false);
        };

        self.compactions.push(report.clone());
        if !report.changed {
            return Ok(false);
        }

        match repair {
            PersistentRepair::Rebuild => {
                self.model_messages = compacted_messages.to_vec();
                self.persistent_messages =
                    persistent_messages_from_model_messages(&self.model_messages);
            }
            PersistentRepair::ReplaceAfter => {
                replace_after_compaction(
                    compacted_messages,
                    &mut self.model_messages,
                    &mut self.persistent_messages,
                    self.current_user_message_id,
                    &[],
                )?;
            }
        }
        self.current_turn_messages_start =
            current_turn_start(&self.model_messages, self.current_user_message_id);
        Ok(true)
    }

    pub(crate) fn finish(
        self,
        host: &mut PluginWorkflowHostMut<'_>,
        output_text: String,
        metadata: Value,
    ) -> Result<PluginWorkflowOutput, PluginWorkflowError> {
        let output = AgentOutput::new(output_text, metadata);
        emit_event(
            host,
            &Event::TurnFinished {
                output: output.clone(),
            },
        )?;
        let new_messages_start =
            current_turn_start(&self.persistent_messages, self.current_user_message_id);
        Ok(PluginWorkflowOutput {
            output,
            messages: self.persistent_messages,
            new_messages_start: Some(new_messages_start),
            compactions: self.compactions,
        })
    }
}
