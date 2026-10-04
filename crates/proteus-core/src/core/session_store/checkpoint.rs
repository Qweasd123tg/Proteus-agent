use super::*;
use crate::contracts::WorkflowToolResultBinding;

impl SessionStore {
    pub(crate) async fn checkpoint_history(
        &self,
        thread_id: ThreadId,
        turn_id: TurnId,
        messages: &[CanonicalMessage],
        compaction: Option<HistoryCompactionReport>,
        tool_results: Vec<WorkflowToolResultBinding>,
    ) -> Result<()> {
        self.mutate_history(
            thread_id,
            Some(turn_id),
            messages.to_vec(),
            HistoryMutationKind::Checkpoint,
            compaction,
            tool_results,
        )
        .await
    }
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
