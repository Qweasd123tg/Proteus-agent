use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

pub fn read_secret_from_config(
    config: &Value,
    default_env: &str,
    json_key: &str,
) -> Result<String> {
    if let Some(value) = config.get("api_key").and_then(Value::as_str) {
        return Ok(value.to_owned());
    }

    if let Some(path) = config.get("api_key_file").and_then(Value::as_str) {
        let key = config
            .get("api_key_json_key")
            .and_then(Value::as_str)
            .unwrap_or(json_key);
        return read_secret_from_json_file(path, key);
    }

    let api_key_env = config
        .get("api_key_env")
        .and_then(Value::as_str)
        .unwrap_or(default_env);
    std::env::var(api_key_env)
        .with_context(|| format!("environment variable {api_key_env} is required"))
}

fn read_secret_from_json_file(path: impl AsRef<Path>, key: &str) -> Result<String> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read secret file {}", path.display()))?;
    let json: Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse secret file {}", path.display()))?;
    json.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("secret key '{key}' is missing in {}", path.display()))
}
