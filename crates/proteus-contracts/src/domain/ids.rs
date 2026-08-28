use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionId(Uuid);

impl ExecutionId {
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub const fn into_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for ExecutionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

pub type SessionId = Uuid;
pub type ThreadId = Uuid;
pub type TurnId = Uuid;
pub type MessageId = Uuid;
pub type PartId = Uuid;
pub type RecordId = Uuid;
pub type ExchangeId = Uuid;
pub type CallId = String;
pub type EventId = Uuid;

pub fn new_session_id() -> SessionId {
    Uuid::new_v4()
}

pub fn new_thread_id() -> ThreadId {
    Uuid::new_v4()
}

pub fn new_turn_id() -> TurnId {
    Uuid::new_v4()
}

pub fn new_execution_id() -> ExecutionId {
    ExecutionId(Uuid::new_v4())
}

pub fn new_message_id() -> MessageId {
    Uuid::new_v4()
}

pub fn new_part_id() -> PartId {
    Uuid::new_v4()
}

pub fn new_record_id() -> RecordId {
    Uuid::new_v4()
}

pub fn new_exchange_id() -> ExchangeId {
    Uuid::new_v4()
}

pub fn new_call_id() -> CallId {
    Uuid::new_v4().to_string()
}

pub fn new_event_id() -> EventId {
    Uuid::new_v4()
}
