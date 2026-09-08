use std::collections::HashSet;

use anyhow::{Result, ensure};

use crate::{
    contracts::WorkflowToolResultBinding,
    domain::{ToolCall, ToolResult},
    model_standard::{CanonicalMessage, ContentPart},
};

/// An explicitly declared ordered result suffix. Completion order may differ
/// from call order; stable identities and insertion positions preserve both
/// ordinary workflow output and cold projection.
#[derive(Debug, Clone, Default)]
pub(crate) struct HistoryCapture {
    base: usize,
    calls: Vec<(ToolCall, WorkflowToolResultBinding, bool)>,
}

impl HistoryCapture {
    pub(crate) fn new(
        history: &[CanonicalMessage],
        bindings: &[WorkflowToolResultBinding],
    ) -> Result<Self> {
        let mut message_ids = history
            .iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        let mut part_ids = history
            .iter()
            .flat_map(|message| message.parts.iter().map(|part| part.part_id))
            .collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        let mut calls = Vec::new();
        for binding in bindings {
            ensure!(
                seen.insert(binding.call_id.clone()),
                "duplicate checkpoint tool binding {}",
                binding.call_id
            );
            ensure!(
                message_ids.insert(binding.message_id) && part_ids.insert(binding.part_id),
                "checkpoint result identities are not unique"
            );
            let mut matching = history
                .iter()
                .flat_map(|message| &message.parts)
                .filter_map(|part| match &part.payload {
                    ContentPart::ToolCall { call } if call.id == binding.call_id => Some(call),
                    _ => None,
                });
            let call = matching.next().ok_or_else(|| {
                anyhow::anyhow!("checkpoint binding {} has no history call", binding.call_id)
            })?;
            ensure!(
                matching.next().is_none(),
                "checkpoint binding has ambiguous history call"
            );
            ensure!(!history.iter().flat_map(|message| &message.parts).any(|part| matches!(&part.payload, ContentPart::ToolResult { result } if result.call_id == binding.call_id)), "checkpoint binding already has a history result");
            calls.push((call.clone(), binding.clone(), false));
        }
        Ok(Self {
            base: history.len(),
            calls,
        })
    }

    pub(crate) fn validate_call(&self, call: &ToolCall) -> Result<()> {
        if let Some((expected, _, _)) = self
            .calls
            .iter()
            .find(|(expected, _, _)| expected.id == call.id)
        {
            ensure!(
                call == expected,
                "executed tool call disagrees with checkpoint history"
            );
        }
        Ok(())
    }

    pub(crate) fn record(
        &mut self,
        history: &mut Vec<CanonicalMessage>,
        result: &ToolResult,
    ) -> Result<bool> {
        let Some(index) = self
            .calls
            .iter()
            .position(|(_, binding, _)| binding.call_id == result.call_id)
        else {
            return Ok(false);
        };
        let offset = self.calls[..index]
            .iter()
            .filter(|(_, _, recorded)| *recorded)
            .count();
        let (_, binding, recorded) = &mut self.calls[index];
        ensure!(!*recorded, "checkpoint received a duplicate tool result");
        ensure!(
            self.base + offset <= history.len(),
            "checkpoint history suffix was lost"
        );
        history.insert(self.base + offset, binding.message(result.clone()));
        *recorded = true;
        Ok(true)
    }
}
