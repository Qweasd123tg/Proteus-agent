use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{Tool, ToolContext, ToolRegistry, ToolSource},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec, ToolSurface},
};

/// Registers provider-executed tools in the same registry as local tools so
/// visibility, safety policy, topology, and duplicate-name checks stay shared.
pub fn register_provider_hosted_tools(
    registry: &mut ToolRegistry,
    provider: &str,
    specs: Vec<ToolSpec>,
) -> Result<()> {
    for spec in specs {
        validate_spec(&spec)?;
        registry.register_with_source(
            ToolSource::ProviderHosted {
                provider: provider.to_owned(),
            },
            ProviderHostedTool { spec },
        )?;
    }
    Ok(())
}

fn validate_spec(spec: &ToolSpec) -> Result<()> {
    let ToolSurface::ProviderHosted { config } = &spec.surface else {
        bail!(
            "model provider returned non-hosted tool '{}' from provider_hosted_tools",
            spec.name
        );
    };
    if spec.name != config.kind().as_str() {
        bail!(
            "provider-hosted tool '{}' must use canonical name '{}'",
            spec.name,
            config.kind().as_str()
        );
    }
    if !matches!(spec.safety, ToolSafety::Network) {
        bail!(
            "provider-hosted tool '{}' must use ToolSafety::Network",
            spec.name
        );
    }
    Ok(())
}

struct ProviderHostedTool {
    spec: ToolSpec,
}

#[async_trait]
impl Tool for ProviderHostedTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> Result<ToolResult> {
        Ok(ToolResult::error(
            call.id.clone(),
            format!(
                "provider-hosted tool '{}' cannot be invoked by the local tool runtime",
                self.spec.name
            ),
        )
        .with_metadata(json!({
            "tool": self.spec.name,
            "provider_hosted": true,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::ToolInvocationOwner,
        domain::{
            HostedToolConfig, WebSearchHostedToolConfig, new_session_id, new_thread_id, new_turn_id,
        },
    };

    fn web_search_spec(safety: ToolSafety) -> ToolSpec {
        ToolSpec::new("web_search", "search", json!({}), safety).with_surface(
            ToolSurface::provider_hosted(HostedToolConfig::WebSearch {
                config: WebSearchHostedToolConfig::default(),
            }),
        )
    }

    #[test]
    fn hosted_registration_rejects_non_network_safety() {
        let spec = web_search_spec(ToolSafety::ReadOnly);
        let error = register_provider_hosted_tools(&mut ToolRegistry::new(), "test", vec![spec])
            .expect_err("hosted execution must remain network-classified");
        assert!(error.to_string().contains("ToolSafety::Network"));
    }

    #[tokio::test]
    async fn registered_hosted_tool_cannot_execute_in_local_runtime() {
        let mut registry = ToolRegistry::new();
        register_provider_hosted_tools(
            &mut registry,
            "openai.responses",
            vec![web_search_spec(ToolSafety::Network)],
        )
        .unwrap();
        let entry = registry.entry("web_search").expect("hosted entry");
        assert_eq!(entry.source.label(), "provider_hosted:openai.responses");

        let call = ToolCall::new("call-hosted", "web_search", json!({}));
        let result = entry
            .tool
            .invoke(
                &call,
                ToolContext::new(
                    std::env::current_dir().unwrap(),
                    ToolInvocationOwner::new(new_session_id(), new_thread_id(), new_turn_id()),
                ),
            )
            .await
            .unwrap();

        assert!(!result.ok);
        assert_eq!(result.call_id, call.id);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("cannot be invoked by the local tool runtime")
        );
        assert_eq!(result.metadata["provider_hosted"], true);
    }
}
