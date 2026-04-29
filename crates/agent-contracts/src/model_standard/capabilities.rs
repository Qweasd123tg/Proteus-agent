use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub supports_tools: bool,
    pub supports_parallel_tool_calls: bool,
    pub supports_streaming: bool,
    pub supports_json_schema: bool,
    pub supports_system_role: bool,
    pub supports_developer_role: bool,
    pub supports_cache_hints: bool,
    pub supports_reasoning_config: bool,
    pub supports_image_input: bool,
    pub supports_file_input: bool,
    pub max_input_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
}

impl ModelCapabilities {
    pub fn basic_text_and_tools() -> Self {
        Self {
            supports_tools: true,
            supports_parallel_tool_calls: false,
            supports_streaming: false,
            supports_json_schema: false,
            supports_system_role: true,
            supports_developer_role: true,
            supports_cache_hints: false,
            supports_reasoning_config: false,
            supports_image_input: false,
            supports_file_input: false,
            max_input_tokens: Some(16_000),
            max_output_tokens: Some(2_048),
        }
    }
}
