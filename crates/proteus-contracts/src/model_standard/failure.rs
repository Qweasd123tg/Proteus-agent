use serde::{Deserialize, Serialize};

use super::CanonicalMessage;

/// Provider-neutral causes that an algorithm can act on without parsing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ModelFailureKind {
    ContextWindowExceeded,
    /// An established response stream disconnected before its terminal event.
    /// The calling algorithm decides whether and how to request continuation.
    StreamDisconnected,
    /// A transient provider failure. The calling algorithm owns the retry budget.
    /// Advice is a delay from receipt, as distinct from a transport/header deadline.
    Retryable {
        retry_delay_ms: Option<u64>,
    },
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
    /// May contain completed tool calls, but never partial deltas or tool results.
    /// The workflow owns execution and explicitly chooses retained history.
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
        let retryable = ModelFailure::new(
            ModelFailureKind::Retryable {
                retry_delay_ms: Some(11054),
            },
            "temporary provider limit",
        );
        let value = serde_json::to_value(&retryable).unwrap();
        assert_eq!(value["kind"]["retryable"]["retry_delay_ms"], 11054);
        assert_eq!(
            serde_json::from_value::<ModelFailure>(value.clone()).unwrap(),
            retryable
        );
        for advice in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("100"),
        ] {
            let mut malformed = value.clone();
            malformed["kind"]["retryable"]["retry_delay_ms"] = advice;
            assert!(serde_json::from_value::<ModelFailure>(malformed).is_err());
        }
        let mut unknown = value;
        unknown["kind"]["retryable"]["provider_advice"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ModelFailure>(unknown).is_err());
        let no_advice = ModelFailure::new(
            ModelFailureKind::Retryable {
                retry_delay_ms: None,
            },
            "transient",
        );
        let value = serde_json::to_value(&no_advice).unwrap();
        assert!(value["kind"]["retryable"]["retry_delay_ms"].is_null());
        assert_eq!(
            serde_json::from_value::<ModelFailure>(value).unwrap(),
            no_advice
        );
    }
}
