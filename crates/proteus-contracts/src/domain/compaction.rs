//! Typed evidence for history compaction and user message derivation.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompactionUserMessageReplacement {
    /// Accepted message retained in the journal, before compaction.
    pub source_message_id: crate::domain::MessageId,
    /// Fresh identity for its compacted representation in model history.
    pub replacement_message_id: crate::domain::MessageId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HistoryCompactionReport {
    pub changed: bool,
    pub user_message_replacements: Vec<CompactionUserMessageReplacement>,
    pub reason: Option<String>,
    pub input_messages: usize,
    pub output_messages: usize,
    pub original_token_estimate: Option<u32>,
    pub output_token_estimate: Option<u32>,
    pub trigger_tokens: Option<u32>,
    pub summary_source: Option<String>,
    pub skipped_reason: Option<String>,
    pub summary: Option<String>,
    pub metadata: serde_json::Value,
}

impl HistoryCompactionReport {
    pub fn unchanged(input_messages: usize, reason: Option<String>) -> Self {
        Self {
            changed: false,
            user_message_replacements: Vec::new(),
            reason,
            input_messages,
            output_messages: input_messages,
            original_token_estimate: None,
            output_token_estimate: None,
            trigger_tokens: None,
            summary_source: None,
            skipped_reason: None,
            summary: None,
            metadata: serde_json::Value::Null,
        }
    }
}
