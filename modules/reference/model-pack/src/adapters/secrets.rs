use anyhow::{Context, Result};
use serde_json::Value;

fn expand_user_path(path: &str) -> std::path::PathBuf {
    expand_user_path_with_home(path, std::env::var_os("HOME").as_deref())
}

fn expand_user_path_with_home(path: &str, home: Option<&std::ffi::OsStr>) -> std::path::PathBuf {
    if let Some(home) = home {
        for prefix in ["~", "$HOME", "${HOME}"] {
            if path == prefix {
                return std::path::PathBuf::from(home);
            }
            if let Some(suffix) = path.strip_prefix(&format!("{prefix}/")) {
                return std::path::PathBuf::from(home).join(suffix);
            }
        }
    }
    std::path::PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_secret_paths_preserve_all_supported_home_forms() {
        let home = std::ffi::OsStr::new("/fixture-home");
        for prefix in ["~", "$HOME", "${HOME}"] {
            assert_eq!(
                expand_user_path_with_home(prefix, Some(home)),
                std::path::PathBuf::from(home)
            );
            assert_eq!(
                expand_user_path_with_home(&format!("{prefix}/secrets/key.json"), Some(home)),
                std::path::PathBuf::from("/fixture-home/secrets/key.json")
            );
            assert_eq!(
                expand_user_path_with_home(prefix, None),
                std::path::PathBuf::from(prefix)
            );
        }
    }
}

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

pub fn read_config_string_or_default(
    config: &Value,
    field: &str,
    default_value: &str,
    default_json_key: &str,
) -> Result<String> {
    if let Some(value) = config.get(field).and_then(Value::as_str) {
        return Ok(value.to_owned());
    }

    let file_field = format!("{field}_file");
    if let Some(path) = config.get(&file_field).and_then(Value::as_str) {
        let json_key_field = format!("{field}_json_key");
        let key = config
            .get(&json_key_field)
            .and_then(Value::as_str)
            .unwrap_or(default_json_key);
        return read_secret_from_json_file(path, key);
    }

    let env_field = format!("{field}_env");
    if let Some(env_name) = config.get(&env_field).and_then(Value::as_str) {
        return std::env::var(env_name)
            .with_context(|| format!("environment variable {env_name} is required"));
    }

    Ok(default_value.to_owned())
}

fn read_secret_from_json_file(path: &str, key: &str) -> Result<String> {
    let path = expand_user_path(path);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read secret file {}", path.display()))?;
    let json: Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse secret file {}", path.display()))?;
    json.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("secret key '{key}' is missing in {}", path.display()))
}
