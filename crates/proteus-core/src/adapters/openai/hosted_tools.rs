use std::collections::BTreeSet;

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::domain::{
    FileSearchHostedToolConfig, HostedToolConfig, HostedToolKind, ToolSafety, ToolSpec,
    ToolSurface, WebSearchContextSize, WebSearchHostedToolConfig,
};

#[derive(Debug, Clone, Default)]
pub(super) struct OpenAiHostedToolsProfile {
    supported_kinds: Vec<HostedToolKind>,
    specs: Vec<ToolSpec>,
    max_tool_calls: Option<u32>,
}

impl OpenAiHostedToolsProfile {
    pub(super) fn from_provider_config(config: &Value, capabilities: &Value) -> Result<Self> {
        let supported_kinds = parse_supported_kinds(capabilities)?;
        let Some(raw) = config.get("hosted_tools") else {
            return Ok(Self {
                supported_kinds,
                ..Self::default()
            });
        };
        let raw: RawHostedTools = serde_json::from_value(raw.clone())
            .map_err(|error| anyhow::anyhow!("invalid openai hosted_tools config: {error}"))?;

        if raw.max_tool_calls == Some(0) {
            bail!("openai hosted_tools.max_tool_calls must be greater than zero");
        }

        let mut specs = Vec::new();
        if let Some(web_search) = raw.web_search {
            require_capability(&supported_kinds, HostedToolKind::WebSearch)?;
            validate_domains("allowed_domains", &web_search.allowed_domains)?;
            validate_domains("blocked_domains", &web_search.blocked_domains)?;
            specs.push(hosted_spec(HostedToolConfig::WebSearch {
                config: WebSearchHostedToolConfig::new(
                    web_search.search_context_size,
                    web_search.allowed_domains,
                    web_search.blocked_domains,
                    web_search.external_web_access,
                    web_search.include_sources,
                ),
            }));
        }
        if let Some(file_search) = raw.file_search {
            require_capability(&supported_kinds, HostedToolKind::FileSearch)?;
            validate_vector_store_ids(&file_search.vector_store_ids)?;
            if file_search.max_num_results == Some(0) {
                bail!("openai hosted_tools.file_search.max_num_results must be greater than zero");
            }
            specs.push(hosted_spec(HostedToolConfig::FileSearch {
                config: FileSearchHostedToolConfig::new(
                    file_search.vector_store_ids,
                    file_search.max_num_results,
                    file_search.include_results,
                ),
            }));
        }
        if raw.max_tool_calls.is_some() && specs.is_empty() {
            bail!("openai hosted_tools.max_tool_calls requires at least one enabled hosted tool");
        }

        Ok(Self {
            supported_kinds,
            specs,
            max_tool_calls: raw.max_tool_calls,
        })
    }

    pub(super) fn supported_kinds(&self) -> &[HostedToolKind] {
        &self.supported_kinds
    }

    pub(super) fn specs(&self) -> Vec<ToolSpec> {
        self.specs.clone()
    }

    pub(super) fn max_tool_calls(&self) -> Option<u32> {
        self.max_tool_calls
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawHostedTools {
    #[serde(default)]
    max_tool_calls: Option<u32>,
    #[serde(default)]
    web_search: Option<RawWebSearch>,
    #[serde(default)]
    file_search: Option<RawFileSearch>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawWebSearch {
    #[serde(default)]
    search_context_size: Option<WebSearchContextSize>,
    #[serde(default)]
    include_sources: bool,
    #[serde(default)]
    allowed_domains: Vec<String>,
    #[serde(default)]
    blocked_domains: Vec<String>,
    #[serde(default)]
    external_web_access: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawFileSearch {
    #[serde(default)]
    vector_store_ids: Vec<String>,
    #[serde(default)]
    max_num_results: Option<u32>,
    #[serde(default)]
    include_results: bool,
}

fn parse_supported_kinds(capabilities: &Value) -> Result<Vec<HostedToolKind>> {
    let Some(value) = capabilities.get("hosted_tools") else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("openai capabilities.hosted_tools must be an array"))?;
    let mut kinds = BTreeSet::new();
    for value in values {
        let value = value.as_str().ok_or_else(|| {
            anyhow::anyhow!("openai capabilities.hosted_tools entries must be strings")
        })?;
        let kind = match value {
            "web_search" => HostedToolKind::WebSearch,
            "file_search" => HostedToolKind::FileSearch,
            other => bail!("openai capabilities.hosted_tools contains unsupported kind '{other}'"),
        };
        if !kinds.insert(kind) {
            bail!("openai capabilities.hosted_tools contains duplicate '{value}'");
        }
    }
    Ok(kinds.into_iter().collect())
}

fn require_capability(supported: &[HostedToolKind], kind: HostedToolKind) -> Result<()> {
    if supported.contains(&kind) {
        Ok(())
    } else {
        bail!(
            "openai hosted_tools.{} requires capabilities.hosted_tools to include '{}'",
            kind.as_str(),
            kind.as_str()
        )
    }
}

fn validate_domains(field: &str, domains: &[String]) -> Result<()> {
    if domains.len() > 100 {
        bail!("openai hosted_tools.web_search.{field} supports at most 100 domains");
    }
    let mut unique = BTreeSet::new();
    for domain in domains {
        let trimmed = domain.trim();
        if trimmed.is_empty() || trimmed.contains("://") || trimmed.contains('/') {
            bail!("openai hosted_tools.web_search.{field} entries must be bare non-empty domains");
        }
        if trimmed != domain {
            bail!(
                "openai hosted_tools.web_search.{field} entries must not contain surrounding whitespace"
            );
        }
        if !unique.insert(domain) {
            bail!("openai hosted_tools.web_search.{field} contains duplicate '{domain}'");
        }
    }
    Ok(())
}

fn validate_vector_store_ids(ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        bail!("openai hosted_tools.file_search.vector_store_ids must not be empty");
    }
    let mut unique = BTreeSet::new();
    for id in ids {
        if id.trim().is_empty() || id.trim() != id {
            bail!(
                "openai hosted_tools.file_search.vector_store_ids entries must be non-empty and trimmed"
            );
        }
        if !unique.insert(id) {
            bail!("openai hosted_tools.file_search.vector_store_ids contains duplicate '{id}'");
        }
    }
    Ok(())
}

fn hosted_spec(config: HostedToolConfig) -> ToolSpec {
    let kind = config.kind();
    let description = match kind {
        HostedToolKind::WebSearch => {
            "Search the web through the model provider and return source-backed current information."
        }
        HostedToolKind::FileSearch => {
            "Search configured provider-managed vector stores and cite matching files."
        }
        _ => "Use a configured tool executed by the model provider.",
    };
    ToolSpec::new(
        kind.as_str(),
        description,
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        ToolSafety::Network,
    )
    .with_surface(ToolSurface::provider_hosted(config))
    .with_metadata(json!({
        "category": "provider_hosted",
        "provider": "openai.responses",
        "hot": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_tools_require_capability_and_validate_config_strictly() {
        let missing = OpenAiHostedToolsProfile::from_provider_config(
            &json!({ "hosted_tools": { "web_search": {} } }),
            &json!({}),
        )
        .expect_err("enabled tool without capability must fail");
        assert!(missing.to_string().contains("capabilities.hosted_tools"));

        let unknown = OpenAiHostedToolsProfile::from_provider_config(
            &json!({ "hosted_tools": { "web_search": { "legacy_domains": [] } } }),
            &json!({ "hosted_tools": ["web_search"] }),
        )
        .expect_err("unknown nested config must fail");
        assert!(unknown.to_string().contains("unknown field"));
    }

    #[test]
    fn configured_specs_are_network_provider_hosted_tools() {
        let profile = OpenAiHostedToolsProfile::from_provider_config(
            &json!({
                "hosted_tools": {
                    "max_tool_calls": 2,
                    "web_search": { "search_context_size": "low", "include_sources": true },
                    "file_search": { "vector_store_ids": ["vs_1"], "max_num_results": 3 }
                }
            }),
            &json!({ "hosted_tools": ["web_search", "file_search"] }),
        )
        .unwrap();

        assert_eq!(profile.max_tool_calls(), Some(2));
        assert_eq!(profile.specs().len(), 2);
        assert!(profile.specs().iter().all(|spec| {
            matches!(spec.safety, ToolSafety::Network)
                && matches!(spec.surface, ToolSurface::ProviderHosted { .. })
        }));
    }
}
