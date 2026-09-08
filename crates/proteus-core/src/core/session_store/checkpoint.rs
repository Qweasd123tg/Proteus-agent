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
        let mut writer = self.writer.lock().await;
        self.materialize_for_write().await?;
        initialize_writer_state(&self.session_dir, self.session_id, &mut writer)?;
        let previous_revision = writer.history_revision();
        append_record(
            &self.session_dir,
            self.session_id,
            JournalRecordAttribution::chat(thread_id, Some(turn_id)),
            JournalEntry::HistoryMutated(HistoryMutated {
                previous_revision,
                new_revision: previous_revision.saturating_add(1),
                mutation: HistoryMutationKind::Checkpoint,
                messages: messages.to_vec(),
                compaction,
                tool_results,
            }),
            self.blob_threshold_bytes,
            &mut writer,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
