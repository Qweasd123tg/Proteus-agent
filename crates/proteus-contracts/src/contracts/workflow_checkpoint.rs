use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    contracts::WorkflowHistoryUpdate,
    domain::{CallId, MessageId, PartId, ToolCall, ToolResult, new_message_id, new_part_id},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

pub const WORKFLOW_HOST_CHECKPOINT_HISTORY_METHOD: &str = "host.history.checkpoint";

/// A workflow explicitly opts these calls into durable conversation history.
/// The host fills the reserved identities only with an actually recorded result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowToolResultBinding {
    pub call_id: CallId,
    /// Exact operation selected by the workflow for this history call. Its id
    /// must equal call_id; its name/arguments may differ from the model input.
    /// This declaration grants no authority: execution still checks the target
    /// registry, policy and safety through the ordinary host tool path.
    pub execution_call: ToolCall,
    pub message_id: MessageId,
    pub part_id: PartId,
}

impl WorkflowToolResultBinding {
    pub fn new(execution_call: ToolCall) -> Self {
        Self {
            call_id: execution_call.id.clone(),
            execution_call,
            message_id: new_message_id(),
            part_id: new_part_id(),
        }
    }

    pub fn message(&self, result: ToolResult) -> CanonicalMessage {
        let mut message =
            CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                .with_tool_call_id(self.call_id.clone());
        message.id = self.message_id;
        message.parts[0].part_id = self.part_id;
        message
    }
}

/// Cumulative update relative to the invocation's input history, just like its
/// terminal output. A checkpoint is committed before its acknowledgement.
/// During model streaming a workflow may extend the model prefix while keeping
/// identical pending result bindings. The host carries already recorded results
/// to the end of that prefix in binding order. Once drained, results appear in
/// `history` with the reserved identities and their bindings are removed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowHistoryCheckpoint {
    pub history: WorkflowHistoryUpdate,
    pub tool_results: Vec<WorkflowToolResultBinding>,
}

#[async_trait]
pub trait WorkflowHistoryRecorder: Send + Sync {
    async fn checkpoint(&self, checkpoint: WorkflowHistoryCheckpoint) -> Result<()>;
}

/// For callers which do not provide conversation persistence (e.g. a replay
/// harness). Runtime turns install the same recorder for every workflow export.
#[derive(Default)]
pub struct NoopWorkflowHistoryRecorder;

#[async_trait]
impl WorkflowHistoryRecorder for NoopWorkflowHistoryRecorder {
    async fn checkpoint(&self, _checkpoint: WorkflowHistoryCheckpoint) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checkpoint_binding_requires_an_explicit_execution_call() {
        let binding = WorkflowToolResultBinding::new(ToolCall::new("call", "probe", json!({})));
        let mut value = serde_json::to_value(&binding).unwrap();
        assert_eq!(
            serde_json::from_value::<WorkflowToolResultBinding>(value.clone()).unwrap(),
            binding
        );
        value.as_object_mut().unwrap().remove("execution_call");
        assert!(
            serde_json::from_value::<WorkflowToolResultBinding>(value)
                .unwrap_err()
                .to_string()
                .contains("execution_call")
        );
    }
}
