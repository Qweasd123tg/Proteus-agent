//! Bounded in-process queue for one running process peer.
//!
//! Delivery into a process peer happens only at explicit app-server turn
//! boundaries.

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
};

use anyhow::{Result, anyhow, bail};
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard, Notify};

use crate::contracts::AgentControlMessage;

const MAX_MAILBOX_MESSAGES: usize = 32;
const MAX_MAILBOX_BYTES: usize = 64_000;

#[derive(Default)]
pub(super) struct ChildMailbox {
    state: Mutex<MailboxState>,
    delivery: AsyncMutex<()>,
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

    /// Serializes mailbox drain with transport delivery. Explicit process
    /// cancel closes the queue first and then waits for this guard, so once
    /// cancel returns no previously drained envelope can still reach the peer.
    pub(super) async fn lock_delivery(&self) -> AsyncMutexGuard<'_, ()> {
        self.delivery.lock().await
    }

    pub(super) async fn wait_for_delivery_idle(&self) {
        let _guard = self.delivery.lock().await;
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

        let mailbox = ChildMailbox::default();
        for _ in 0..4 {
            mailbox
                .enqueue(message("x".repeat(MAX_AGENT_MESSAGE_BYTES)))
                .expect("within aggregate byte cap");
        }
        assert!(
            mailbox.enqueue(message("one-byte-over-cap")).is_err(),
            "aggregate byte cap must apply before the count cap"
        );
    }

    #[tokio::test]
    async fn cancel_waits_for_an_active_delivery_guard() {
        let mailbox = std::sync::Arc::new(ChildMailbox::default());
        let guard = mailbox.lock_delivery().await;
        let waiting = {
            let mailbox = mailbox.clone();
            tokio::spawn(async move {
                mailbox.wait_for_delivery_idle().await;
            })
        };
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .expect("cancel-side wait must finish after delivery releases")
            .expect("wait task");
    }
}
