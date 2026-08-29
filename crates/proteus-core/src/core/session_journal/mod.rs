mod projection;
mod recorder;
mod storage;
mod types;

pub use projection::JournalProjection;
pub use recorder::{SessionAgentToolRecorder, SessionExecutionRecorder};
pub use storage::{DEFAULT_BLOB_THRESHOLD_BYTES, JOURNAL_FILE};
pub use types::*;

pub(crate) use storage::{
    JournalRecordAttribution, JournalWriterState, append_record, initialize_writer_state,
    journal_path, load_records,
};

#[cfg(test)]
mod tests;
