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
    current_user_index, persistent_messages_from_model_messages, replace_after_compaction,
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
    history_replacement_len: Option<usize>,
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
        let persistent_messages = input.history.clone();
        let current_user_message = persistent_messages.last().ok_or_else(|| {
            PluginWorkflowError::new(
                "workflow input history must end with the persisted current user message",
            )
        })?;
        if current_user_message.role != MessageRole::User
            || message_text(current_user_message) != input.task.text
        {
            return Err(PluginWorkflowError::new(
                "workflow input history does not end with the current task user message",
            ));
        }
        let current_user_message_id = current_user_message.id;

        // Provider prompt caches reuse an unchanged request prefix. Keep the
        // ephemeral workspace context before durable conversation history so
        // the next turn can extend `context + history` instead of inserting
        // new conversation messages in front of the context.
        let mut model_messages =
            Vec::with_capacity(bundle.chunks.len() + persistent_messages.len());
        for chunk in bundle.chunks {
            model_messages.push(
                CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
                    .with_name(CONTEXT_MESSAGE_NAME),
            );
        }
        model_messages.extend(persistent_messages.iter().cloned());
        let current_turn_messages_start = model_messages.len();

        Ok(Self {
            model_messages,
            persistent_messages,
            current_user_message_id,
            current_turn_messages_start,
            context_chunks,
            context_token_estimate,
            compactions: Vec::new(),
            history_replacement_len: None,
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
        let current_user_index =
            current_user_index(&self.model_messages, self.current_user_message_id).ok_or_else(
                || {
                    PluginWorkflowError::new(
                        "compaction changed history but dropped the current user message",
                    )
                },
            )?;
        self.current_turn_messages_start = current_user_index + 1;
        self.history_replacement_len = Some(self.persistent_messages.len());
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
        let mut history_prefix = self.persistent_messages;
        let (history_replacement, new_messages) = match self.history_replacement_len {
            Some(replacement_len) => {
                if replacement_len > history_prefix.len() {
                    return Err(PluginWorkflowError::new(
                        "workflow history replacement boundary is beyond persistent history",
                    ));
                }
                let new_messages = history_prefix.split_off(replacement_len);
                (Some(history_prefix), new_messages)
            }
            None => {
                let current_user_position =
                    current_user_index(&history_prefix, self.current_user_message_id).ok_or_else(
                        || {
                            PluginWorkflowError::new(
                                "workflow persistent history dropped the current user message",
                            )
                        },
                    )?;
                let new_messages = history_prefix.split_off(current_user_position + 1);
                (None, new_messages)
            }
        };
        Ok(PluginWorkflowOutput {
            output,
            new_messages,
            history_replacement,
            compactions: self.compactions,
        })
    }
}

fn message_text(message: &CanonicalMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
