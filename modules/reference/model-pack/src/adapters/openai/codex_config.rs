//! Subscription settings are explicit; typos must not silently select API behavior.
use anyhow::{Context, Result, bail};
use serde_json::Value;

pub(super) fn validate(config: &Value) -> Result<()> {
    let object = config
        .as_object()
        .context("openai_codex config must be an object")?;
    let capability_keys = [
        "supports_parallel_tool_calls",
        "supports_freeform_tools",
        "supports_json_schema",
        "supports_reasoning_config",
        "support_verbosity",
        "hosted_tools",
    ];
    for key in object.keys() {
        if !capability_keys.contains(&key.as_str())
            && ![
                "implementation",
                "auth_file",
                "oauth_issuer",
                "base_url",
                "stream",
                "stream_error_fallback",
                "http1_only",
                "max_input_tokens",
                "capabilities",
                "verbosity",
                "default_verbosity",
                "service_tier",
                "store",
                "item_ids_enabled",
                "client_metadata",
                "prompt_cache",
                "prompt_cache_key",
                "request_max_retries",
                "stream_idle_timeout_ms",
            ]
            .contains(&key.as_str())
        {
            bail!("unknown openai_codex setting {key}");
        }
    }
    if let Some(capabilities) = object.get("capabilities") {
        for key in capabilities
            .as_object()
            .context("openai_codex capabilities must be an object")?
            .keys()
        {
            if !capability_keys.contains(&key.as_str()) {
                bail!("unknown openai_codex capability {key}");
            }
        }
    }
    for key in ["stream", "prompt_cache"] {
        if object.get(key).is_some_and(|value| !value.is_boolean()) {
            bail!("openai_codex {key} must be a boolean");
        }
    }
    if let Some(value) = object.get("max_input_tokens") {
        if !value
            .as_u64()
            .is_some_and(|n| n > 0 && n <= u32::MAX as u64)
        {
            bail!("openai_codex max_input_tokens must be a positive u32");
        }
    }
    if object
        .get("prompt_cache_key")
        .is_some_and(|value| value.as_str().is_none_or(|s| s.trim().is_empty()))
    {
        bail!("openai_codex prompt_cache_key must be a non-empty string");
    }
    Ok(())
}
