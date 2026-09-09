//! Invocation-scoped model delivery for workflow modules. The host owns model
//! IO, presentation and cancellation; the workflow owns item handling/tools.
use serde::{Deserialize, Serialize};

use crate::model_standard::{CanonicalMessage, CanonicalModelResponse, ModelFailure};

pub const WORKFLOW_HOST_START_MODEL_STREAM_METHOD: &str = "host.model.stream.start";
pub const WORKFLOW_HOST_NEXT_MODEL_STREAM_METHOD: &str = "host.model.stream.next";

/// One active cursor per workflow invocation. Start takes the same request DTO
/// as complete. Next consumes one completed item or the terminal outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowModelStreamCursor {
    pub stream_id: uuid::Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowModelStreamItem {
    MessageCompleted { message: CanonicalMessage },
    Response { response: CanonicalModelResponse },
    Error { failure: ModelFailure },
}
