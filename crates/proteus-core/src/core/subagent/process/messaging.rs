//! Agent-control envelope delivery over one leased peer's stdio connection.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use proteus_contracts::app_protocol::StdioRequest;
use serde_json::Value;

use super::child::ChildProcess;
use crate::{
    contracts::AgentControlMessage,
    domain::{AgentOutput, new_call_id},
};

pub(super) enum PeerMessageResponse {
    Queued,
    TurnCompleted(AgentOutput),
}

#[derive(Default)]
pub(super) struct PeerMessageDelivery {
    pending_receipts: HashSet<String>,
}

impl PeerMessageDelivery {
    pub(super) fn is_settled(&self) -> bool {
        self.pending_receipts.is_empty()
    }

    pub(super) async fn queue(
        &mut self,
        child: &mut ChildProcess,
        messages: Vec<AgentControlMessage>,
    ) -> Result<()> {
        for message in messages {
            message.validate()?;
            let request_id = new_call_id();
            child
                .send(&StdioRequest::Send {
                    id: Some(request_id.clone()),
                    text: message.model_text(),
                })
                .await?;
            self.pending_receipts.insert(request_id);
        }
        Ok(())
    }

    /// Starts a new peer turn inside the same logical agent generation after
    /// the previous peer turn won the terminal race. Remaining envelopes are
    /// queued as steering for the new turn.
    pub(super) async fn start_continuation(
        &mut self,
        child: &mut ChildProcess,
        messages: Vec<AgentControlMessage>,
    ) -> Result<String> {
        let mut messages = messages.into_iter();
        let first = messages
            .next()
            .expect("continuation requires at least one mailbox message");
        first.validate()?;
        let request_id = new_call_id();
        child
            .send(&StdioRequest::Send {
                id: Some(request_id.clone()),
                text: first.model_text(),
            })
            .await?;
        self.queue(child, messages.collect()).await?;
        Ok(request_id)
    }

    /// A generic app-server `send` either queues steering into the active turn
    /// and answers immediately, or starts a new turn when the peer has already
    /// become idle. The latter response carries a full `AgentOutput` and must
    /// supersede the previous terminal result instead of being discarded as an
    /// acknowledgement.
    pub(super) fn confirm(
        &mut self,
        id: &str,
        ok: bool,
        output: Option<&Value>,
        error: Option<&str>,
    ) -> Result<Option<PeerMessageResponse>> {
        if !self.pending_receipts.remove(id) {
            return Ok(None);
        }
        if !ok {
            bail!(
                "subagent child rejected agent-control message: {}",
                error.unwrap_or("unknown error")
            );
        }
        let output = output.ok_or_else(|| {
            anyhow::anyhow!("subagent child returned an empty agent-control response")
        })?;
        if output.get("queued").and_then(Value::as_bool) == Some(true) {
            if output.get("accepted").and_then(Value::as_bool) != Some(true) {
                bail!("subagent child returned an invalid steering acknowledgement");
            }
            return Ok(Some(PeerMessageResponse::Queued));
        }
        let output = serde_json::from_value::<AgentOutput>(output.clone())
            .context("subagent child returned an invalid agent-control turn output")?;
        Ok(Some(PeerMessageResponse::TurnCompleted(output)))
    }
}
