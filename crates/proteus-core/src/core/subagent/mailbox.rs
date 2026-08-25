//! Bounded in-process mailbox for one running child loop.
//!
//! Delivery happens only at explicit model/tool boundaries in `child_loop`.
//! `close_and_drain` makes the terminal boundary atomic with enqueue: a
//! message is either rejected because the child is already idle, or retained
//! in the child history before its resumable snapshot is written.

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
};

use anyhow::{Result, anyhow, bail};
use tokio::sync::Notify;

use crate::contracts::AgentControlMessage;

const MAX_MAILBOX_MESSAGES: usize = 32;
const MAX_MAILBOX_BYTES: usize = 64_000;

#[derive(Default)]
pub(super) struct ChildMailbox {
    state: Mutex<MailboxState>,
    notify: Notify,
}

#[derive(Default)]
struct MailboxState {
    closed: bool,
    queued_bytes: usize,
    messages: VecDeque<AgentControlMessage>,
}

impl ChildMailbox {
    pub(super) fn enqueue(&self, message: AgentControlMessage) -> Result<usize> {
        message.validate()?;
        let message_bytes = message.content.len();

        let mut state = self.lock()?;
        if state.closed {
            bail!("subagent mailbox is closed; use followup_task to start another turn");
        }
        if state.messages.len() >= MAX_MAILBOX_MESSAGES
            || state.queued_bytes.saturating_add(message_bytes) > MAX_MAILBOX_BYTES
        {
            bail!(
                "subagent mailbox capacity reached ({MAX_MAILBOX_MESSAGES} messages / {MAX_MAILBOX_BYTES} bytes)"
            );
        }
        state.queued_bytes += message_bytes;
        state.messages.push_back(message);
        let queued = state.messages.len();
        drop(state);
        self.notify.notify_one();
        Ok(queued)
    }

    pub(super) fn drain(&self) -> Result<Vec<AgentControlMessage>> {
        let mut state = self.lock()?;
        Ok(drain_locked(&mut state))
    }

    pub(super) async fn notified(&self) {
        self.notify.notified().await;
    }

    /// At a would-be successful terminal boundary, either drains messages and
    /// keeps the loop addressable, or closes the empty mailbox atomically.
    pub(super) fn drain_or_close(&self) -> Result<Vec<AgentControlMessage>> {
        let mut state = self.lock()?;
        if state.messages.is_empty() {
            state.closed = true;
            return Ok(Vec::new());
        }
        Ok(drain_locked(&mut state))
    }

    /// Closes every non-success terminal path and returns messages accepted
    /// just before the boundary so the runner can persist them in history.
    pub(super) fn close_and_drain(&self) -> Result<Vec<AgentControlMessage>> {
        let mut state = self.lock()?;
        state.closed = true;
        Ok(drain_locked(&mut state))
    }

    /// Explicit cancel wins over queued steering: no new delivery may start
    /// after cancellation has been accepted by the coordinator.
    pub(super) fn close_and_discard(&self) -> Result<usize> {
        let mut state = self.lock()?;
        state.closed = true;
        let discarded = state.messages.len();
        let _ = drain_locked(&mut state);
        Ok(discarded)
    }

    fn lock(&self) -> Result<MutexGuard<'_, MailboxState>> {
        self.state
            .lock()
            .map_err(|_| anyhow!("subagent mailbox lock poisoned"))
    }
}

fn drain_locked(state: &mut MailboxState) -> Vec<AgentControlMessage> {
    state.queued_bytes = 0;
    state.messages.drain(..).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{AgentAddress, MAX_AGENT_MESSAGE_BYTES};

    fn message(content: impl Into<String>) -> AgentControlMessage {
        AgentControlMessage::from_root(AgentAddress::child("worker").unwrap(), content).unwrap()
    }

    #[test]
    fn terminal_boundary_does_not_lose_accepted_messages() {
        let mailbox = ChildMailbox::default();
        assert_eq!(mailbox.enqueue(message("first")).expect("enqueue"), 1);
        assert_eq!(mailbox.drain_or_close().expect("drain")[0].content, "first");
        assert_eq!(mailbox.enqueue(message("second")).expect("enqueue"), 1);
        assert_eq!(
            mailbox.drain_or_close().expect("drain")[0].content,
            "second"
        );
        assert!(mailbox.drain_or_close().expect("close").is_empty());
        assert!(mailbox.enqueue(message("late")).is_err());
    }

    #[test]
    fn mailbox_enforces_message_and_total_caps() {
        let mailbox = ChildMailbox::default();
        assert!(
            AgentControlMessage::from_root(AgentAddress::child("worker").unwrap(), "").is_err()
        );
        assert!(
            AgentControlMessage::from_root(
                AgentAddress::child("worker").unwrap(),
                "x".repeat(MAX_AGENT_MESSAGE_BYTES + 1)
            )
            .is_err()
        );
        for index in 0..MAX_MAILBOX_MESSAGES {
            mailbox
                .enqueue(message(format!("message-{index}")))
                .expect("within count cap");
        }
        assert!(mailbox.enqueue(message("overflow")).is_err());
    }
}
