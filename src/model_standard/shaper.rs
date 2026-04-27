use anyhow::Result;

use crate::{
    domain::{CacheHints, ReasoningConfig, ToolChoice},
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
        }

        if !capabilities.supports_cache_hints {
            request.cache = CacheHints::default();
        }

        if !capabilities.supports_reasoning_config {
            request.reasoning = ReasoningConfig::default();
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
