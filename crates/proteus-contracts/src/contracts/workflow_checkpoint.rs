use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    contracts::WorkflowHistoryUpdate,
    domain::{CallId, MessageId, PartId, ToolResult, new_message_id, new_part_id},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

pub const WORKFLOW_HOST_CHECKPOINT_HISTORY_METHOD: &str = "host.history.checkpoint";

/// A workflow explicitly opts these calls into durable conversation history.
/// The host fills the reserved identities only with an actually recorded result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowToolResultBinding {
    pub call_id: CallId,
    pub message_id: MessageId,
    pub part_id: PartId,
}

impl WorkflowToolResultBinding {
    pub fn new(call_id: CallId) -> Self {
        Self {
            call_id,
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
