use super::*;
use crate::contracts::WorkflowToolResultBinding;

impl SessionStore {
    #[cfg(test)]
    pub(crate) async fn pause_next_append(
        &self,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        std::sync::mpsc::Sender<bool>,
    ) {
        self.writer.lock().await.pause_next_append()
    }

    /// Once admitted, the worker owns the lock until commit or rollback. Dropping
    /// the caller only stops waiting; it cannot interrupt a durable transaction.
    async fn write_transaction<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Self, &mut JournalWriterState) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let mut writer = self.writer.clone().lock_owned().await;
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            store.materialize_for_write()?;
            initialize_writer_state(&store.session_dir, store.session_id, &mut writer)?;
            operation(&store, &mut writer)
        })
        .await
        .context("session journal transaction worker failed")?
    }

    /// Admission may write image bytes before the first journal record. Create
    /// the canonical session identity and acquire its writer lease first.
    pub(crate) async fn prepare_attachments(&self) -> Result<()> {
        self.write_transaction(|_, _| Ok(())).await
    }

    fn materialize_for_write(&self) -> Result<()> {
        let parent = self.session_dir.parent().ok_or_else(|| {
            anyhow!(
                "session directory has no parent: {}",
                self.session_dir.display()
            )
        })?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create session parent {}", parent.display()))?;
        let created = match std::fs::create_dir(&self.session_dir) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to create session dir {}",
                        self.session_dir.display()
                    )
                });
            }
        };
        ensure_writable_identity(
            &self.session_dir,
            self.session_id,
            self.directory_kind,
            &self.workspace_path()?,
            created,
        )
    }

    pub(super) async fn append_fact(
        &self,
        attribution: JournalRecordAttribution,
        entry: JournalEntry,
    ) -> Result<JournalRecord> {
        self.write_transaction(move |store, writer| {
            append_record(
                &store.session_dir,
                store.session_id,
                attribution,
                entry,
                store.blob_threshold_bytes,
                writer,
            )
        })
        .await
    }

    pub(super) async fn mutate_history(
        &self,
        thread_id: ThreadId,
        turn_id: Option<TurnId>,
        messages: Vec<CanonicalMessage>,
        mutation: HistoryMutationKind,
        compaction: Option<HistoryCompactionReport>,
        tool_results: Vec<WorkflowToolResultBinding>,
    ) -> Result<()> {
        self.write_transaction(move |store, writer| {
            let previous_revision = writer.history_revision();
            append_record(
                &store.session_dir,
                store.session_id,
                JournalRecordAttribution::chat(thread_id, turn_id),
                JournalEntry::HistoryMutated(HistoryMutated {
                    previous_revision,
                    new_revision: previous_revision.saturating_add(1),
                    tool_results,
                    mutation,
                    messages,
                    compaction,
                }),
                store.blob_threshold_bytes,
                writer,
            )?;
            Ok(())
        })
        .await
    }
}
