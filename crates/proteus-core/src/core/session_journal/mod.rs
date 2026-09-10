pub(crate) mod history_capture;
mod projection;
mod recorder;
mod storage;
mod types;
mod usage;

pub(crate) use usage::UsageProjection;

pub use projection::JournalProjection;
pub(crate) use projection::JournalValidationState;
pub use recorder::{SessionExecutionRecorder, SessionToolExecutionRecorder};
pub use storage::{DEFAULT_BLOB_THRESHOLD_BYTES, JOURNAL_FILE};
pub use types::*;

pub(crate) use storage::redacted_history;
pub(crate) use storage::{
    JournalRecordAttribution, JournalWriterState, append_record, initialize_writer_state,
    journal_path, load_records,
};

#[cfg(test)]
mod tests;
