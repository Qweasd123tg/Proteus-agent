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

const MAX_MAILBOX_MESSAGES: usize = 32;
const MAX_MAILBOX_BYTES: usize = 64_000;
const MAX_MESSAGE_BYTES: usize = 16_000;

#[derive(Default)]
pub(super) struct ChildMailbox {
    state: Mutex<MailboxState>,
}

#[derive(Default)]
struct MailboxState {
    closed: bool,
    queued_bytes: usize,
    messages: VecDeque<String>,
}

impl ChildMailbox {
    pub(super) fn enqueue(&self, message: &str) -> Result<usize> {
        let message = message.trim();
        if message.is_empty() {
            bail!("subagent message must not be blank");
        }
        if message.len() > MAX_MESSAGE_BYTES {
            bail!("subagent message exceeds {MAX_MESSAGE_BYTES} bytes");
        }

        let mut state = self.lock()?;
        if state.closed {
            bail!("subagent mailbox is closed; use followup_task to start another turn");
        }
        if state.messages.len() >= MAX_MAILBOX_MESSAGES
            || state.queued_bytes.saturating_add(message.len()) > MAX_MAILBOX_BYTES
        {
            bail!(
                "subagent mailbox capacity reached ({MAX_MAILBOX_MESSAGES} messages / {MAX_MAILBOX_BYTES} bytes)"
            );
        }
        state.queued_bytes += message.len();
        state.messages.push_back(message.to_owned());
        Ok(state.messages.len())
    }

    pub(super) fn drain(&self) -> Result<Vec<String>> {
        let mut state = self.lock()?;
        Ok(drain_locked(&mut state))
    }

    /// At a would-be successful terminal boundary, either drains messages and
    /// keeps the loop addressable, or closes the empty mailbox atomically.
    pub(super) fn drain_or_close(&self) -> Result<Vec<String>> {
        let mut state = self.lock()?;
        if state.messages.is_empty() {
            state.closed = true;
            return Ok(Vec::new());
        }
        Ok(drain_locked(&mut state))
    }

    /// Closes every non-success terminal path and returns messages accepted
    /// just before the boundary so the runner can persist them in history.
    pub(super) fn close_and_drain(&self) -> Result<Vec<String>> {
        let mut state = self.lock()?;
        state.closed = true;
        Ok(drain_locked(&mut state))
    }

    fn lock(&self) -> Result<MutexGuard<'_, MailboxState>> {
        self.state
            .lock()
            .map_err(|_| anyhow!("subagent mailbox lock poisoned"))
    }
}

fn drain_locked(state: &mut MailboxState) -> Vec<String> {
    state.queued_bytes = 0;
    state.messages.drain(..).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_boundary_does_not_lose_accepted_messages() {
        let mailbox = ChildMailbox::default();
        assert_eq!(mailbox.enqueue("first").expect("enqueue"), 1);
        assert_eq!(mailbox.drain_or_close().expect("drain"), vec!["first"]);
        assert_eq!(mailbox.enqueue("second").expect("enqueue"), 1);
        assert_eq!(mailbox.drain_or_close().expect("drain"), vec!["second"]);
        assert!(mailbox.drain_or_close().expect("close").is_empty());
        assert!(mailbox.enqueue("late").is_err());
    }

    #[test]
    fn mailbox_enforces_message_and_total_caps() {
        let mailbox = ChildMailbox::default();
        assert!(mailbox.enqueue("").is_err());
        assert!(mailbox.enqueue(&"x".repeat(MAX_MESSAGE_BYTES + 1)).is_err());
        for index in 0..MAX_MAILBOX_MESSAGES {
            mailbox
                .enqueue(&format!("message-{index}"))
                .expect("within count cap");
        }
        assert!(mailbox.enqueue("overflow").is_err());
    }
}
