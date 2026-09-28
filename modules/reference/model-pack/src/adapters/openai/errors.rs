use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;
use std::{sync::OnceLock, time::Duration};

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
    let mut failure = failure_from_error_value(
        parsed.as_ref().and_then(|value| value.get("error")),
        format!(
            "OpenAI API error {status}: {}",
            String::from_utf8_lossy(&body)
        ),
    );
    // The provider's bounded HTTP retry has finished. Codex's sampling loop
    // still treats InternalServerError as retryable, with its own budget.
    if status == reqwest::StatusCode::INTERNAL_SERVER_ERROR {
        failure.kind = ModelFailureKind::Retryable {
            retry_delay_ms: None,
        };
    }
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
    let fallback_message = fallback_message.into();
    let error = payload
        .get("response")
        .and_then(|response| response.get("error"))
        .and_then(|error| serde_json::from_value::<FailedResponseError>(error.clone()).ok());
    let Some(error) = error else {
        return ModelFailure::new(
            ModelFailureKind::Retryable {
                retry_delay_ms: None,
            },
            fallback_message,
        );
    };
    // This is the pinned Responses failure classification, not the HTTP/error
    // event classification. Unknown response.failed causes are transient.
    let kind = match error.code.as_deref() {
        Some("context_length_exceeded") => ModelFailureKind::ContextWindowExceeded,
        Some(
            "insufficient_quota"
            | "usage_not_included"
            | "cyber_policy"
            | "misalignment_policy_violation"
            | "invalid_prompt"
            | "bio_policy"
            | "server_is_overloaded"
            | "slow_down",
        ) => ModelFailureKind::Other,
        _ => ModelFailureKind::Retryable {
            retry_delay_ms: retry_delay_ms(&error),
        },
    };
    let message = match error.code.as_deref() {
        Some("cyber_policy") => error
            .message
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| {
                "This request has been flagged for possible cybersecurity risk.".into()
            }),
        Some("misalignment_policy_violation") => error
            .message
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| {
                "This request was blocked due to a misalignment policy violation.".into()
            }),
        Some("invalid_prompt" | "bio_policy") => {
            error.message.unwrap_or_else(|| "Invalid request.".into())
        }
        _ => error.message.unwrap_or_default(),
    };
    ModelFailure::new(kind, message)
}

// Validate the same error envelope fields as the pinned Responses parser.
#[derive(Deserialize)]
#[allow(dead_code)]
struct FailedResponseError {
    r#type: Option<String>,
    code: Option<String>,
    message: Option<String>,
    plan_type: Option<String>,
    resets_at: Option<i64>,
    misalignment: Option<Value>,
}

fn retry_delay_ms(error: &FailedResponseError) -> Option<u64> {
    if error.code.as_deref() != Some("rate_limit_exceeded") {
        return None;
    }
    static RE: OnceLock<regex_lite::Regex> = OnceLock::new();
    let regex = RE.get_or_init(|| {
        regex_lite::Regex::new(r"(?i)try again in\s*(\d+(?:\.\d+)?)\s*(s|ms|seconds?)")
            .expect("valid pinned rate-limit regex")
    });
    let captures = regex.captures(error.message.as_deref()?)?;
    let value = captures.get(1)?.as_str().parse::<f64>().ok()?;
    let unit = captures.get(2)?.as_str().to_ascii_lowercase();
    let delay = if unit == "s" || unit.starts_with("second") {
        Duration::try_from_secs_f64(value).ok()?
    } else if unit == "ms" {
        Duration::from_millis(value as u64)
    } else {
        return None;
    };
    // Canonical advice is whole milliseconds; unrepresentable values cannot
    // be carried across the process boundary.
    delay.as_millis().try_into().ok()
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

    #[test]
    fn failed_responses_preserve_pinned_terminal_causes_and_retry_advice() {
        for (code, message, expected) in [
            (
                "context_length_exceeded",
                "full",
                ModelFailureKind::ContextWindowExceeded,
            ),
            ("insufficient_quota", "quota", ModelFailureKind::Other),
            ("usage_not_included", "quota", ModelFailureKind::Other),
            ("cyber_policy", "policy", ModelFailureKind::Other),
            (
                "misalignment_policy_violation",
                "policy",
                ModelFailureKind::Other,
            ),
            ("invalid_prompt", "request", ModelFailureKind::Other),
            ("bio_policy", "request", ModelFailureKind::Other),
            ("server_is_overloaded", "overload", ModelFailureKind::Other),
            ("slow_down", "overload", ModelFailureKind::Other),
            (
                "server_error",
                "Try again in 5s",
                ModelFailureKind::Retryable {
                    retry_delay_ms: None,
                },
            ),
            (
                "rate_limit_exceeded",
                "Try again in 11.054s.",
                ModelFailureKind::Retryable {
                    retry_delay_ms: Some(11054),
                },
            ),
            (
                "rate_limit_exceeded",
                "Please TRY AGAIN IN 150ms.",
                ModelFailureKind::Retryable {
                    retry_delay_ms: Some(150),
                },
            ),
            (
                "rate_limit_exceeded",
                "Try again in 0 seconds.",
                ModelFailureKind::Retryable {
                    retry_delay_ms: Some(0),
                },
            ),
            (
                "rate_limit_exceeded",
                "No advice",
                ModelFailureKind::Retryable {
                    retry_delay_ms: None,
                },
            ),
        ] {
            let failure = failure_from_sse_failed(
                &serde_json::json!({"response": {
                    "error": {"code": code, "message": message}
                }}),
                "fallback",
            );
            assert_eq!(failure.kind, expected, "{code}: {message}");
            assert_eq!(failure.message, message);
        }
        for payload in [
            serde_json::json!({}),
            serde_json::json!({"response": {"error": null}}),
            serde_json::json!({"response": {"error": {"code": "insufficient_quota", "message": 1}}}),
        ] {
            let failure = failure_from_sse_failed(&payload, "response.failed event received");
            assert_eq!(
                failure.kind,
                ModelFailureKind::Retryable {
                    retry_delay_ms: None
                }
            );
            assert_eq!(failure.message, "response.failed event received");
        }
    }
}
