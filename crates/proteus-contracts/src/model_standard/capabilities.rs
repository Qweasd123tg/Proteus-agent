use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
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
    /// Базовая capability: только текст + tools, без streaming/image/etc.
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

    /// Пустые capabilities — всё false. Используется как fallback.
    pub fn empty() -> Self {
        Self {
            supports_tools: false,
            supports_parallel_tool_calls: false,
            supports_streaming: false,
            supports_json_schema: false,
            supports_system_role: false,
            supports_developer_role: false,
            supports_cache_hints: false,
            supports_reasoning_config: false,
            supports_image_input: false,
            supports_file_input: false,
            max_input_tokens: None,
            max_output_tokens: None,
        }
    }

    pub fn with_tools(mut self, value: bool) -> Self {
        self.supports_tools = value;
        self
    }
    pub fn with_parallel_tool_calls(mut self, value: bool) -> Self {
        self.supports_parallel_tool_calls = value;
        self
    }
    pub fn with_streaming(mut self, value: bool) -> Self {
        self.supports_streaming = value;
        self
    }
    pub fn with_json_schema(mut self, value: bool) -> Self {
        self.supports_json_schema = value;
        self
    }
    pub fn with_system_role(mut self, value: bool) -> Self {
        self.supports_system_role = value;
        self
    }
    pub fn with_developer_role(mut self, value: bool) -> Self {
        self.supports_developer_role = value;
        self
    }
    pub fn with_cache_hints(mut self, value: bool) -> Self {
        self.supports_cache_hints = value;
        self
    }
    pub fn with_reasoning_config(mut self, value: bool) -> Self {
        self.supports_reasoning_config = value;
        self
    }
    pub fn with_image_input(mut self, value: bool) -> Self {
        self.supports_image_input = value;
        self
    }
    pub fn with_file_input(mut self, value: bool) -> Self {
        self.supports_file_input = value;
        self
    }
    pub fn with_max_input_tokens(mut self, value: Option<u32>) -> Self {
        self.max_input_tokens = value;
        self
    }
    pub fn with_max_output_tokens(mut self, value: Option<u32>) -> Self {
        self.max_output_tokens = value;
        self
    }
}
