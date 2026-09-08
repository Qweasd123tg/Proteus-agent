use serde::{Deserialize, Serialize};

use super::CanonicalMessage;

/// Provider-neutral causes that an algorithm can act on without parsing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ModelFailureKind {
    ContextWindowExceeded,
    /// An established response stream disconnected before its terminal event.
    /// The calling algorithm decides whether and how to request continuation.
    StreamDisconnected,
    Interrupted,
    SessionBudgetExceeded,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFailure {
    pub kind: ModelFailureKind,
    pub message: String,
    /// Fully completed assistant messages accepted before the model call failed.
    /// Partial deltas and tool calls are never represented here.
    pub completed_messages: Vec<CanonicalMessage>,
}

impl ModelFailure {
    pub fn new(kind: ModelFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            completed_messages: Vec::new(),
        }
    }

    pub fn with_completed_messages(mut self, completed_messages: Vec<CanonicalMessage>) -> Self {
        self.completed_messages = completed_messages;
        self
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::new(ModelFailureKind::Other, message)
    }

    pub fn from_error(error: &anyhow::Error) -> Self {
        error
            .downcast_ref::<Self>()
            .cloned()
            .unwrap_or_else(|| Self::other(format!("{error:#}")))
    }
}

impl std::fmt::Display for ModelFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ModelFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_preserves_cause_through_error_context_and_rejects_unknown_shape() {
        let failure = ModelFailure::new(ModelFailureKind::ContextWindowExceeded, "window full");
        let wrapped = anyhow::Error::new(failure.clone()).context("model callback");
        assert_eq!(ModelFailure::from_error(&wrapped), failure);
        let value = serde_json::to_value(&failure).unwrap();
        assert_eq!(
            serde_json::from_value::<ModelFailure>(value.clone()).unwrap(),
            failure
        );
        let mut unknown = value.clone();
        unknown["kind"] = serde_json::json!("provider_specific");
        assert!(serde_json::from_value::<ModelFailure>(unknown).is_err());
        assert!(
            serde_json::from_value::<ModelFailure>(serde_json::json!({"message":"old"})).is_err()
        );
        let mut missing_progress = value;
        missing_progress
            .as_object_mut()
            .unwrap()
            .remove("completed_messages");
        assert!(serde_json::from_value::<ModelFailure>(missing_progress).is_err());
    }
}
