//! Ordering of complete pending snapshots, independent of a UI framework.

#[derive(Debug, Default)]
pub struct PendingCursor {
    connection: u64,
    revision: Option<(String, String, u64)>,
}

impl PendingCursor {
    /// An SSE open is a new synchronization boundary, even when it reconnects
    /// to the same session. In-flight reads from earlier opens are obsolete.
    pub fn begin_connection(&mut self) {
        self.connection = self
            .connection
            .checked_add(1)
            .expect("connection generation");
        self.revision = None;
    }

    pub fn read_ticket(&self) -> u64 {
        self.connection
    }

    /// The initial stream snapshot establishes the live session incarnation.
    /// Subsequent complete snapshots may skip revisions, but never go back.
    pub fn accept_stream(&mut self, session: &str, stream: &str, seq: u64) -> bool {
        if let Some((current_session, current_stream, current_seq)) = &self.revision {
            if current_session != session || current_stream != stream || seq <= *current_seq {
                return false;
            }
        }
        self.revision = Some((session.to_owned(), stream.to_owned(), seq));
        true
    }

    pub fn accept_read(&mut self, ticket: u64, session: &str, stream: &str, seq: u64) -> bool {
        // A read must not pick the incarnation: it may come from the backend
        // that served a request just before disconnect. The stream is baseline.
        if ticket != self.connection || self.revision.is_none() {
            return false;
        }
        self.accept_stream(session, stream, seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_reads_and_events_cannot_restore_removed_pending_items() {
        let mut cursor = PendingCursor::default();
        cursor.begin_connection();
        let ticket = cursor.read_ticket();
        assert!(!cursor.accept_read(ticket, "session", "live", 4));
        assert!(cursor.accept_stream("session", "live", 4));
        assert!(cursor.accept_stream("session", "live", 7));
        assert!(!cursor.accept_read(ticket, "session", "live", 5));
        assert!(!cursor.accept_stream("session", "live", 7));
        assert!(cursor.accept_read(ticket, "session", "live", 9));
        assert!(!cursor.accept_stream("session", "live", 8));
        assert!(cursor.accept_stream("session", "live", 10));
    }

    #[test]
    fn reconnect_fences_old_reads_and_accepts_a_restarted_session() {
        let mut cursor = PendingCursor::default();
        cursor.begin_connection();
        assert!(cursor.accept_stream("a", "old", 90));
        let old_read = cursor.read_ticket();
        cursor.begin_connection();
        assert!(!cursor.accept_read(old_read, "a", "old", 91));
        assert!(cursor.accept_stream("a", "new", 0));
        let read = cursor.read_ticket();
        assert!(!cursor.accept_read(read, "a", "old", 92));
        assert!(!cursor.accept_stream("a", "old", 93));
        assert!(!cursor.accept_stream("b", "new", 1));
        assert!(cursor.accept_read(read, "a", "new", 1));
        cursor.begin_connection();
        assert!(!cursor.accept_read(read, "a", "new", 2));
        assert!(cursor.accept_stream("b", "other", 0));
    }
}
