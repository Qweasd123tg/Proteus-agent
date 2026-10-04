use super::*;

#[cfg(test)]
#[derive(Debug)]
pub(super) struct AppendPause {
    started: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<bool>,
}

#[cfg(test)]
impl AppendPause {
    fn before_write(self, file: &mut std::fs::File, line: &[u8]) -> Result<()> {
        let _ = self.started.send(());
        let fail_after_partial_write = self.release.recv().context("append pause dropped")?;
        if fail_after_partial_write {
            file.write_all(&line[..line.len() / 2])?;
            bail!("injected partial journal write failure");
        }
        Ok(())
    }
}

#[cfg(test)]
impl JournalWriterState {
    pub(crate) fn pause_next_append(
        &mut self,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        std::sync::mpsc::Sender<bool>,
    ) {
        let (started, pending) = tokio::sync::oneshot::channel();
        let (release, receiver) = std::sync::mpsc::channel();
        self.append_pause = Some(AppendPause {
            started,
            release: receiver,
        });
        (pending, release)
    }

    pub(crate) fn committed_offset(&self) -> u64 {
        self.committed_offset
    }
}

pub(crate) fn append_record(
    session_dir: &Path,
    session_id: SessionId,
    attribution: JournalRecordAttribution,
    entry: JournalEntry,
    blob_threshold_bytes: usize,
    state: &mut JournalWriterState,
) -> Result<JournalRecord> {
    initialize_writer_state(session_dir, session_id, state)?;
    let path = journal_path(session_dir);
    restore_committed_offset(&path, state.committed_offset)?;

    let kind = entry.kind();
    let mut record = JournalRecord {
        schema_version: JOURNAL_SCHEMA_VERSION,
        record_id: new_record_id(),
        session_seq: state.next_seq,
        timestamp_ms: unix_timestamp_ms(),
        session_id,
        execution_id: attribution.execution_id,
        thread_id: attribution.thread_id,
        turn_id: attribution.turn_id,
        entry,
    };
    if let JournalEntry::HookInvoked(trace) = &record.entry {
        crate::core::session_journal::hooks::validate_raw_trace(&record, trace)?;
    }
    let mut payload = record.entry.payload_value()?;
    redact_sensitive_values(&mut payload);
    let payload_bytes = serde_json::to_vec(&payload)?;
    if payload_bytes.len() > MAX_PAYLOAD_BYTES {
        bail!(
            "journal payload is {} bytes, maximum is {} bytes",
            payload_bytes.len(),
            MAX_PAYLOAD_BYTES
        );
    }
    record.entry = JournalEntry::from_kind_and_payload(kind, payload.clone())
        .context("redacted journal payload no longer matches its canonical DTO")?;
    let mut next_validation = state.validation.clone();
    next_validation.apply(&record)?;

    let stored_payload = if payload_bytes.len() >= blob_threshold_bytes {
        write_blob(session_dir, &payload_bytes)?
    } else {
        StoredPayload::Inline { value: payload }
    };
    let stored = StoredJournalRecord {
        schema_version: record.schema_version,
        record_id: record.record_id,
        session_seq: record.session_seq,
        timestamp_ms: record.timestamp_ms,
        session_id: record.session_id,
        execution_id: record.execution_id,
        thread_id: record.thread_id,
        turn_id: record.turn_id,
        kind,
        payload: stored_payload,
    };
    let mut line = serde_json::to_vec(&stored)?;
    line.push(b'\n');
    let next_committed_offset = state
        .committed_offset
        .checked_add(line.len() as u64)
        .ok_or_else(|| anyhow!("journal offset overflow for {}", path.display()))?;
    let mut rollback = AppendRollback::new(path.clone(), state.committed_offset);
    let write_result = (|| {
        let mut file = StdOpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        #[cfg(test)]
        if let Some(pause) = state.append_pause.take() {
            pause.before_write(&mut file, &line)?;
        }
        file.write_all(&line)
            .with_context(|| format!("failed to append {}", path.display()))?;
        file.flush()
            .with_context(|| format!("failed to flush {}", path.display()))?;
        file.sync_data()
            .with_context(|| format!("failed to sync {}", path.display()))?;
        Ok::<(), anyhow::Error>(())
    })();
    if let Err(error) = write_result {
        return match rollback.rollback() {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(anyhow!(
                "{error:#}; journal rollback failed: {rollback_error:#}; next append will retry recovery"
            )),
        };
    }
    rollback.commit();

    state.next_seq = state.next_seq.saturating_add(1);
    state.validation = next_validation;
    state.committed_offset = next_committed_offset;
    state.usage.apply(&record);
    Ok(record)
}
