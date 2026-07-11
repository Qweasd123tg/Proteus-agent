use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::model_standard::ModelCapabilities;

#[derive(Debug, Clone)]
pub(super) struct OpenAiModelProfile {
    pub supports_parallel_tool_calls: bool,
    pub supports_json_schema: bool,
    pub supports_reasoning_config: bool,
    pub support_verbosity: bool,
    pub verbosity: Option<String>,
    pub default_verbosity: Option<String>,
    pub service_tier: Option<String>,
    pub store: bool,
    pub client_metadata: BTreeMap<String, String>,
}

impl OpenAiModelProfile {
    pub fn from_provider_config(config: &Value) -> Result<Self> {
        let capabilities = config.get("capabilities").unwrap_or(&Value::Null);
        if !capabilities.is_null() && !capabilities.is_object() {
            bail!("openai capabilities must be an object");
        }

        let support_verbosity = bool_setting(config, capabilities, "support_verbosity", false)?;
        let verbosity = optional_enum(config, "verbosity", &["low", "medium", "high"])?;
        let default_verbosity =
            optional_enum(config, "default_verbosity", &["low", "medium", "high"])?;
        if (verbosity.is_some() || default_verbosity.is_some()) && !support_verbosity {
            bail!("openai verbosity/default_verbosity requires support_verbosity = true");
        }

        let store = optional_bool(config, "store")?.unwrap_or(false);
        let item_ids_enabled = optional_bool(config, "item_ids_enabled")?.unwrap_or(false);
        if store || item_ids_enabled {
            bail!(
                "openai store/item_ids_enabled require provider item ids in canonical history, which Proteus does not support yet"
            );
        }

        Ok(Self {
            supports_parallel_tool_calls: bool_setting(
                config,
                capabilities,
                "supports_parallel_tool_calls",
                false,
            )?,
            supports_json_schema: bool_setting(
                config,
                capabilities,
                "supports_json_schema",
                false,
            )?,
            supports_reasoning_config: bool_setting(
                config,
                capabilities,
                "supports_reasoning_config",
                false,
            )?,
            support_verbosity,
            verbosity,
            default_verbosity,
            service_tier: optional_non_empty_string(config, "service_tier")?,
            store: false,
            client_metadata: string_map(config, "client_metadata")?,
        })
    }

    pub fn capabilities(&self, max_input_tokens: Option<u32>) -> ModelCapabilities {
        ModelCapabilities::empty()
            .with_tools(true)
            .with_parallel_tool_calls(self.supports_parallel_tool_calls)
            .with_json_schema(self.supports_json_schema)
            .with_system_role(true)
            .with_developer_role(true)
            .with_cache_hints(true)
            .with_reasoning_config(self.supports_reasoning_config)
            .with_streaming(true)
            .with_max_input_tokens(max_input_tokens)
    }

    pub fn effective_verbosity(&self) -> Option<&str> {
        if self.support_verbosity {
            self.verbosity
                .as_deref()
                .or(self.default_verbosity.as_deref())
        } else {
            None
        }
    }
}

fn bool_setting(config: &Value, capabilities: &Value, key: &str, default: bool) -> Result<bool> {
    if let Some(value) = capabilities.get(key) {
        return value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("openai capabilities.{key} must be a boolean"));
    }
    Ok(optional_bool(config, key)?.unwrap_or(default))
}

fn optional_bool(config: &Value, key: &str) -> Result<Option<bool>> {
    config
        .get(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("openai {key} must be a boolean"))
        })
        .transpose()
}

fn optional_non_empty_string(config: &Value, key: &str) -> Result<Option<String>> {
    config
        .get(key)
        .map(|value| {
            let value = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("openai {key} must be a string"))?
                .trim();
            if value.is_empty() {
                bail!("openai {key} must not be empty");
            }
            Ok(value.to_owned())
        })
        .transpose()
}

fn optional_enum(config: &Value, key: &str, allowed: &[&str]) -> Result<Option<String>> {
    let value = optional_non_empty_string(config, key)?;
    if let Some(value) = value.as_deref()
        && !allowed.contains(&value)
    {
        bail!("openai {key} must be one of: {}", allowed.join(", "));
    }
    Ok(value)
}

fn string_map(config: &Value, key: &str) -> Result<BTreeMap<String, String>> {
    let Some(value) = config.get(key) else {
        return Ok(BTreeMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("openai {key} must be an object"))?;
    object
        .iter()
        .map(|(name, value)| {
            let value = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("openai {key}.{name} must be a string"))?;
            Ok((name.clone(), value.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn unknown_model_profile_is_conservative() {
        let profile = OpenAiModelProfile::from_provider_config(&json!({})).unwrap();

        assert!(!profile.supports_parallel_tool_calls);
        assert!(!profile.supports_json_schema);
        assert!(!profile.supports_reasoning_config);
        assert_eq!(profile.effective_verbosity(), None);
        assert!(!profile.store);
    }

    #[test]
    fn explicit_model_capabilities_and_controls_are_parsed() {
        let profile = OpenAiModelProfile::from_provider_config(&json!({
            "capabilities": {
                "supports_parallel_tool_calls": true,
                "supports_json_schema": true,
                "supports_reasoning_config": true
            },
            "support_verbosity": true,
            "default_verbosity": "low",
            "service_tier": "priority",
            "client_metadata": { "session_id": "session-1" }
        }))
        .unwrap();

        assert!(profile.supports_parallel_tool_calls);
        assert!(profile.supports_json_schema);
        assert!(profile.supports_reasoning_config);
        assert_eq!(profile.effective_verbosity(), Some("low"));
        assert_eq!(profile.service_tier.as_deref(), Some("priority"));
        assert_eq!(profile.client_metadata["session_id"], "session-1");
    }

    #[test]
    fn verbosity_without_capability_is_rejected() {
        let error = OpenAiModelProfile::from_provider_config(&json!({
            "verbosity": "high"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("support_verbosity"));
    }

    #[test]
    fn store_and_item_ids_fail_closed_until_history_can_preserve_ids() {
        for config in [
            json!({ "store": true }),
            json!({ "item_ids_enabled": true }),
        ] {
            let error = OpenAiModelProfile::from_provider_config(&config).unwrap_err();
            assert!(error.to_string().contains("provider item ids"));
        }
    }
}
