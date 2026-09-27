use serde::{Deserialize, Serialize};

/// Ordered, provider-neutral model-context facts preceding this root invocation.
/// The workflow owns how these facts affect its next compaction estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelContextObservation {
    /// Consecutive provider usage observations may be coalesced. `last_tokens`
    /// is the most recent response within this group.
    Usage { total_tokens: u64, last_tokens: u32 },
    /// An ordinary model request exceeded a known raw context window.
    ContextWindowExceeded { max_input_tokens: u32 },
    /// A changed history compaction was accepted by the host.
    HistoryCompacted,
}
