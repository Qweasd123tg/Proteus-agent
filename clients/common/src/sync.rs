//! Session stream ordering, independent of transport and UI framework.
use proteus_contracts::{app_protocol::AppServerEvent, domain::SessionId};

#[derive(Debug, Default)]
pub struct SessionCursor {
    revision: Option<(SessionId, String, u64)>,
    awaiting_snapshot: bool,
}

impl SessionCursor {
    pub fn begin_connection(&mut self) {
        self.revision = None;
        self.awaiting_snapshot = true;
    }

    /// A connection or lag must establish a complete baseline before deltas.
    /// Pending has its own revision; activity may address background sessions.
    pub fn accept(&mut self, event: &AppServerEvent) -> bool {
        match event {
            AppServerEvent::SessionSnapshot { snapshot } => {
                if let Some((session, stream, seq)) = &self.revision
                    && (*session != snapshot.session_id
                        || stream != &snapshot.stream_id
                        || snapshot.seq < *seq
                        || (snapshot.seq == *seq && !self.awaiting_snapshot))
                {
                    return false;
                }
                self.revision = Some((
                    snapshot.session_id,
                    snapshot.stream_id.clone(),
                    snapshot.seq,
                ));
                self.awaiting_snapshot = false;
                true
            }
            AppServerEvent::EventStreamLagged { .. } => {
                self.awaiting_snapshot = true;
                true
            }
            AppServerEvent::PendingRequestsUpdated { .. }
            | AppServerEvent::SessionActivityUpdated { .. }
            | AppServerEvent::Error { .. }
            | AppServerEvent::Shutdown => true,
            AppServerEvent::Runtime { envelope } => {
                !self.awaiting_snapshot
                    && self
                        .revision
                        .as_ref()
                        .is_some_and(|(session, _, _)| *session == envelope.session_id)
            }
            _ => !self.awaiting_snapshot && self.revision.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::{app_protocol::AppSessionSnapshot, domain::new_session_id};

    #[test]
    fn reconnect_and_lag_require_a_current_baseline() {
        let session_id = new_session_id();
        let snapshot = |stream: &str, seq| AppServerEvent::SessionSnapshot {
            snapshot: Box::new(AppSessionSnapshot {
                session_id,
                stream_id: stream.into(),
                seq,
                root_thread_id: None,
                transcript: vec![],
                execution: Default::default(),
            }),
        };
        let delta = AppServerEvent::UserMessageSubmitted {
            text: "hello".into(),
        };
        let mut cursor = SessionCursor::default();
        cursor.begin_connection();
        assert!(!cursor.accept(&delta));
        assert!(cursor.accept(&snapshot("first", 3)));
        assert!(cursor.accept(&delta));
        let foreign = AppServerEvent::Runtime {
            envelope: Box::new(proteus_contracts::domain::EventEnvelope::new(
                proteus_contracts::domain::EventContext::new(
                    new_session_id(),
                    proteus_contracts::domain::new_thread_id(),
                    None,
                ),
                0,
                proteus_contracts::domain::Event::AssistantReasoningDelta {
                    text: "foreign".into(),
                },
            )),
        };
        assert!(!cursor.accept(&foreign));
        assert!(!cursor.accept(&snapshot("first", 3)));
        assert!(!cursor.accept(&snapshot("foreign", 4)));
        assert!(cursor.accept(&AppServerEvent::EventStreamLagged { count: 2 }));
        assert!(!cursor.accept(&delta));
        assert!(!cursor.accept(&snapshot("first", 2)));
        assert!(cursor.accept(&snapshot("first", 6)));
        assert!(cursor.accept(&delta));
        // Background activity can lag without changing this session's seq.
        // Its replacement baseline must still release the delta gate.
        assert!(cursor.accept(&AppServerEvent::EventStreamLagged { count: 1 }));
        assert!(cursor.accept(&snapshot("first", 6)));
        assert!(cursor.accept(&delta));
        cursor.begin_connection();
        assert!(!cursor.accept(&delta));
        assert!(cursor.accept(&snapshot("restarted", 0)));
        assert!(cursor.accept(&delta));
    }
}
