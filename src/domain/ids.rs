use uuid::Uuid;

pub type SessionId = Uuid;
pub type MessageId = Uuid;
pub type CallId = String;
pub type EventId = Uuid;

pub fn new_session_id() -> SessionId {
    Uuid::new_v4()
}

pub fn new_message_id() -> MessageId {
    Uuid::new_v4()
}

pub fn new_call_id() -> CallId {
    Uuid::new_v4().to_string()
}

pub fn new_event_id() -> EventId {
    Uuid::new_v4()
}
