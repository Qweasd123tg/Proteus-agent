use anyhow::Result;
use serde_json::Value;

use crate::model_standard::{ModelFailure, ModelFailureKind};

const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;

pub(super) async fn ensure_success(mut response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let mut body = Vec::new();
    while body.len() < MAX_ERROR_BODY_BYTES {
        let Some(chunk) = response.chunk().await? else {
            break;
        };
        let remaining = MAX_ERROR_BODY_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    let parsed = serde_json::from_slice::<Value>(&body).ok();
    let failure = failure_from_error_value(
        parsed.as_ref().and_then(|value| value.get("error")),
        format!(
            "OpenAI API error {status}: {}",
            String::from_utf8_lossy(&body)
        ),
    );
    Err(anyhow::Error::new(failure))
}

pub(super) fn failure_from_error_value(
    error: Option<&Value>,
    fallback_message: impl Into<String>,
) -> ModelFailure {
    let fallback_message = fallback_message.into();
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or(&fallback_message)
        .to_owned();
    let kind = error
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .filter(|code| *code == "context_length_exceeded")
        .map(|_| ModelFailureKind::ContextWindowExceeded)
        .unwrap_or(ModelFailureKind::Other);
    ModelFailure::new(kind, message)
}

pub(super) fn failure_from_sse_error(
    payload: &Value,
    fallback_message: impl Into<String>,
) -> ModelFailure {
    failure_from_error_value(payload.get("error").or(Some(payload)), fallback_message)
}

pub(super) fn failure_from_sse_failed(
    payload: &Value,
    fallback_message: impl Into<String>,
) -> ModelFailure {
    let response = payload.get("response").unwrap_or(payload);
    failure_from_error_value(response.get("error"), fallback_message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_documented_openai_error_code_is_actionable() {
        let context = failure_from_error_value(
            Some(&serde_json::json!({"code": "context_length_exceeded", "message": "too long"})),
            "fallback",
        );
        assert_eq!(context.kind, ModelFailureKind::ContextWindowExceeded);

        let arbitrary = failure_from_error_value(
            Some(&serde_json::json!({"code": "rate_limit_exceeded", "message": "slow down"})),
            "fallback",
        );
        assert_eq!(arbitrary.kind, ModelFailureKind::Other);
    }
}
