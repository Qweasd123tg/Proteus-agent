use serde::{Deserialize, Serialize};

use crate::domain::{ExecutionId, SessionId, ThreadId, TurnId};

/// Optional application/chat projection attached to a generic execution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnAttribution {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
}

impl AgentTurnAttribution {
    pub fn new(session_id: SessionId, thread_id: ThreadId, turn_id: TurnId) -> Self {
        Self {
            session_id,
            thread_id,
            turn_id,
        }
    }
}

/// Durable/runtime attribution for one logical execution.
///
/// `ExecutionId` is always present. Conversational identities are an optional
/// projection supplied by the agent layer; detached execution never invents
/// Session/Thread/Turn values.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAttribution {
    pub execution_id: ExecutionId,
    pub agent: Option<AgentTurnAttribution>,
}

impl ExecutionAttribution {
    pub fn detached(execution_id: ExecutionId) -> Self {
        Self {
            execution_id,
            agent: None,
        }
    }

    pub fn for_turn(
        execution_id: ExecutionId,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> Self {
        Self {
            execution_id,
            agent: Some(AgentTurnAttribution::new(session_id, thread_id, turn_id)),
        }
    }
}
