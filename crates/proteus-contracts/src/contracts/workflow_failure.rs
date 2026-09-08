use serde::{Deserialize, Serialize};

use crate::{
    domain::HistoryCompactionReport,
    model_standard::{CanonicalMessage, ModelFailure},
};

/// Persistent progress explicitly returned by a workflow that could not finish.
/// Uses the same append/replacement semantics as a successful WorkflowOutput.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowHistoryUpdate {
    pub new_messages: Vec<CanonicalMessage>,
    pub history_replacement: Option<Vec<CanonicalMessage>>,
    pub compactions: Vec<HistoryCompactionReport>,
}

impl WorkflowHistoryUpdate {
    pub fn new(new_messages: Vec<CanonicalMessage>) -> Self {
        Self {
            new_messages,
            history_replacement: None,
            compactions: Vec::new(),
        }
    }

    pub fn with_history_replacement(mut self, messages: Vec<CanonicalMessage>) -> Self {
        self.history_replacement = Some(messages);
        self
    }

    pub fn with_compactions(mut self, compactions: Vec<HistoryCompactionReport>) -> Self {
        self.compactions = compactions;
        self
    }
}

/// An algorithm error can preserve known progress without claiming success.
/// A missing history means no progress was returned, not that no effects occurred.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowFailure {
    pub message: String,
    pub history: Option<WorkflowHistoryUpdate>,
    pub model_failure: Option<ModelFailure>,
}

impl WorkflowFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            history: None,
            model_failure: None,
        }
    }

    pub fn with_history(mut self, history: WorkflowHistoryUpdate) -> Self {
        self.history = Some(history);
        self
    }
}

impl std::fmt::Display for WorkflowFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WorkflowFailure {}
