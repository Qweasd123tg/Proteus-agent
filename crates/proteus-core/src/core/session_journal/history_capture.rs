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
    pub(crate) fn has_binding(&self, binding: &WorkflowToolResultBinding) -> bool {
        self.calls
            .iter()
            .any(|(_, existing, _)| existing == binding)
    }

    /// Extend the model prefix while retaining the already committed result
    /// suffix. Only an identical binding may carry a result across checkpoints.
    /// This is shared by live history and cold journal projection.
    pub(crate) fn rebase(
        &self,
        current: &[CanonicalMessage],
        proposed: &[CanonicalMessage],
        bindings: &[WorkflowToolResultBinding],
        compacted: bool,
    ) -> Result<(Self, Vec<CanonicalMessage>)> {
        let mut next = Self::new(proposed, bindings)?;
        let mut history = proposed.to_vec();
        for (_, binding, _) in &self.calls {
            if let Some(reused) = bindings.iter().find(|item| item.call_id == binding.call_id) {
                ensure!(
                    reused == binding,
                    "checkpoint changed a pending tool result binding"
                );
            }
        }
        if compacted || proposed.starts_with(current) {
            return Ok((next, history));
        }
        let recorded = self
            .calls
            .iter()
            .filter(|(_, _, recorded)| *recorded)
            .count();
        ensure!(
            self.base + recorded == current.len() && proposed.starts_with(&current[..self.base]),
            "checkpoint discarded committed history without a new compaction"
        );
        for ((_, binding, _), message) in self
            .calls
            .iter()
            .filter(|(_, _, recorded)| *recorded)
            .zip(&current[self.base..])
        {
            ensure!(
                next.has_binding(binding),
                "checkpoint discarded a committed tool result"
            );
            let result = message
                .parts
                .iter()
                .find_map(|part| match &part.payload {
                    ContentPart::ToolResult { result } => Some(result),
                    _ => None,
                })
                .ok_or_else(|| anyhow::anyhow!("checkpoint result suffix was changed"))?;
            ensure!(
                &binding.message(result.clone()) == message,
                "checkpoint result suffix was changed"
            );
            next.record(&mut history, result)?;
        }
        Ok((next, history))
    }

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
                binding.execution_call.id == binding.call_id,
                "checkpoint execution call id disagrees with history binding"
            );
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
            matching.next().ok_or_else(|| {
                anyhow::anyhow!("checkpoint binding {} has no history call", binding.call_id)
            })?;
            ensure!(
                matching.next().is_none(),
                "checkpoint binding has ambiguous history call"
            );
            ensure!(!history.iter().flat_map(|message| &message.parts).any(|part| matches!(&part.payload, ContentPart::ToolResult { result } if result.call_id == binding.call_id)), "checkpoint binding already has a history result");
            calls.push((binding.execution_call.clone(), binding.clone(), false));
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
                "executed tool call disagrees with checkpoint execution binding"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_standard::MessageRole;
    use serde_json::json;

    #[test]
    fn rebasing_preserves_exact_results_and_bindings_in_call_order() {
        let calls = [
            ToolCall::new("a", "probe", json!({})),
            ToolCall::new("b", "probe", json!({})),
        ];
        let mut prefix = vec![CanonicalMessage::new(
            MessageRole::Assistant,
            calls
                .iter()
                .cloned()
                .map(|call| ContentPart::ToolCall { call })
                .collect(),
        )];
        let bindings = calls
            .into_iter()
            .map(WorkflowToolResultBinding::new)
            .collect::<Vec<_>>();
        let mut capture = HistoryCapture::new(&prefix, &bindings).unwrap();
        let mut history = prefix.clone();
        let results = [
            ToolResult::ok("a".into(), "first"),
            ToolResult::ok("b".into(), "second"),
        ];
        capture.record(&mut history, &results[1]).unwrap();
        capture.record(&mut history, &results[0]).unwrap();
        prefix.push(CanonicalMessage::text(
            MessageRole::Assistant,
            "later model item",
        ));
        let (capture, rebased) = capture.rebase(&history, &prefix, &bindings, false).unwrap();
        assert_eq!(&rebased[..prefix.len()], &prefix);
        assert_eq!(
            &rebased[prefix.len()..],
            &[
                bindings[0].message(results[0].clone()),
                bindings[1].message(results[1].clone())
            ]
        );
        assert!(
            capture
                .rebase(&rebased, &prefix, &bindings[..1], false)
                .is_err()
        );
        let mut changed = bindings.clone();
        changed[0].part_id = crate::domain::new_part_id();
        assert!(capture.rebase(&rebased, &prefix, &changed, false).is_err());
        let (_, drained) = capture.rebase(&rebased, &rebased, &[], false).unwrap();
        assert_eq!(drained, rebased);
        assert!(capture.rebase(&rebased, &[], &[], false).is_err());
    }
}
