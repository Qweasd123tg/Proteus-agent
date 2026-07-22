use anyhow::{Result, bail};

use crate::{
    domain::{CacheHints, ReasoningConfig, ResponseFormat, ToolChoice, ToolSafety, ToolSurface},
    model_standard::{CanonicalModelRequest, ModelCapabilities},
};

#[derive(Debug, Default, Clone)]
pub struct RequestShaper;

impl RequestShaper {
    pub fn shape(
        &self,
        mut request: CanonicalModelRequest,
        capabilities: &ModelCapabilities,
    ) -> Result<CanonicalModelRequest> {
        if !capabilities.supports_tools {
            request.tools.clear();
            request.tool_choice = ToolChoice::None;
        } else {
            if !capabilities.supports_freeform_tools {
                let unsupported = request
                    .tools
                    .iter()
                    .filter(|tool| matches!(tool.surface, ToolSurface::Freeform { .. }))
                    .map(|tool| format!("'{}'", tool.name))
                    .collect::<Vec<_>>();
                if !unsupported.is_empty() {
                    bail!(
                        "model '{}' does not support freeform tools: {}",
                        request.model.model,
                        unsupported.join(", ")
                    );
                }
            }

            for tool in &request.tools {
                let Some(kind) = tool.surface.hosted_kind() else {
                    continue;
                };
                if tool.name != kind.as_str() {
                    bail!(
                        "provider-hosted tool '{}' must use canonical name '{}'",
                        tool.name,
                        kind.as_str()
                    );
                }
                if !matches!(tool.safety, ToolSafety::Network) {
                    bail!(
                        "provider-hosted tool '{}' must use ToolSafety::Network",
                        tool.name
                    );
                }
                if !capabilities.provider_hosted_tools.contains(&kind) {
                    bail!(
                        "model '{}' does not support provider-hosted tool '{}'",
                        request.model.model,
                        kind.as_str()
                    );
                }
            }
        }

        if !capabilities.supports_cache_hints {
            request.cache = CacheHints::default();
        }

        if !capabilities.supports_reasoning_config {
            request.reasoning = ReasoningConfig::default();
        }

        if matches!(request.response_format, ResponseFormat::JsonSchema { .. })
            && !capabilities.supports_json_schema
        {
            bail!(
                "model '{}' does not support JSON schema responses",
                request.model.model
            );
        }

        if let Some(max_input_tokens) = capabilities.max_input_tokens {
            request.limits.max_input_tokens = Some(
                request
                    .limits
                    .max_input_tokens
                    .map_or(max_input_tokens, |limit| limit.min(max_input_tokens)),
            );
        }
        if let Some(max_output_tokens) = capabilities.max_output_tokens {
            request.limits.max_output_tokens = Some(
                request
                    .limits
                    .max_output_tokens
                    .map_or(max_output_tokens, |limit| limit.min(max_output_tokens)),
            );
        }

        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        domain::{
            HostedToolConfig, HostedToolKind, ModelRef, ToolSafety, ToolSpec, ToolSurface,
            WebSearchHostedToolConfig,
        },
        model_standard::{CanonicalMessage, MessageRole},
    };

    #[test]
    fn freeform_tools_require_explicit_model_capability() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("openai", "custom-proxy-model"),
            vec![CanonicalMessage::text(MessageRole::User, "edit")],
        )
        .with_tools(vec![
            ToolSpec::new(
                "apply_patch",
                "Apply a patch",
                json!({}),
                ToolSafety::WritesFiles,
            )
            .with_surface(ToolSurface::freeform_lark("start: \"*** Begin Patch\"")),
        ]);

        let error = RequestShaper
            .shape(
                request.clone(),
                &ModelCapabilities::empty().with_tools(true),
            )
            .expect_err("freeform must be opt-in");
        assert!(
            error
                .to_string()
                .contains("does not support freeform tools")
        );
        assert!(error.to_string().contains("apply_patch"));

        let shaped = RequestShaper
            .shape(
                request,
                &ModelCapabilities::empty()
                    .with_tools(true)
                    .with_freeform_tools(true),
            )
            .expect("explicit freeform capability");
        assert!(matches!(
            shaped.tools[0].surface,
            ToolSurface::Freeform { .. }
        ));
    }

    #[test]
    fn strict_json_schema_requires_model_capability() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("openai", "unknown-model"),
            vec![CanonicalMessage::text(MessageRole::User, "answer")],
        )
        .with_response_format(ResponseFormat::JsonSchema {
            name: "answer".to_owned(),
            schema: json!({ "type": "object" }),
            strict: true,
        });

        let error = RequestShaper
            .shape(request.clone(), &ModelCapabilities::empty())
            .unwrap_err();
        assert!(error.to_string().contains("does not support JSON schema"));

        let shaped = RequestShaper
            .shape(request, &ModelCapabilities::empty().with_json_schema(true))
            .unwrap();
        assert!(matches!(
            shaped.response_format,
            ResponseFormat::JsonSchema { strict: true, .. }
        ));
    }

    #[test]
    fn unsupported_cache_hints_drop_routing_key_with_the_hints() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("local", "model"),
            vec![CanonicalMessage::text(MessageRole::User, "answer")],
        )
        .with_cache(CacheHints::new(true, true).with_routing_key("session-1"));

        let shaped = RequestShaper
            .shape(request, &ModelCapabilities::empty())
            .unwrap();

        assert_eq!(shaped.cache, CacheHints::default());
    }

    #[test]
    fn provider_hosted_tools_require_explicit_model_capability() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("openai", "custom-proxy-model"),
            vec![CanonicalMessage::text(MessageRole::User, "search")],
        )
        .with_tools(vec![
            ToolSpec::new(
                "web_search",
                "Search the web",
                json!({ "type": "object" }),
                ToolSafety::Network,
            )
            .with_surface(ToolSurface::provider_hosted(HostedToolConfig::WebSearch {
                config: WebSearchHostedToolConfig::default(),
            })),
        ]);

        let error = RequestShaper
            .shape(
                request.clone(),
                &ModelCapabilities::empty().with_tools(true),
            )
            .expect_err("provider-hosted tools must be capability-gated");
        assert!(
            error
                .to_string()
                .contains("does not support provider-hosted tool 'web_search'")
        );

        let shaped = RequestShaper
            .shape(
                request,
                &ModelCapabilities::empty()
                    .with_tools(true)
                    .with_provider_hosted_tools(vec![HostedToolKind::WebSearch]),
            )
            .expect("explicit hosted-tool capability");
        assert!(matches!(
            shaped.tools[0].surface,
            ToolSurface::ProviderHosted { .. }
        ));
    }
}
