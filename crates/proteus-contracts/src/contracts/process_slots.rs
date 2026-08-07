//! Strict process-v1 DTOs for slots whose canonical Rust traits do not carry
//! their own wire representation.
//!
//! The host chooses the contract from the slot, never from `module_id`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    contracts::ToolInvocationOwner,
    domain::{
        AgentOutput, AgentTask, ContextBundle, ContextChunk, MemoryItem, MemoryQuery, Patch,
        PatchResult, PolicyDecision, ToolCall, ToolResult, ToolSpec,
    },
};

use super::ToolExposureInput;

pub const PROCESS_MEMORY_CONTRACT_VERSION: &str = "v1";
pub const PROCESS_MEMORY_REMEMBER_METHOD: &str = "remember";
pub const PROCESS_MEMORY_RECALL_METHOD: &str = "recall";

pub const PROCESS_PATCH_CONTRACT_VERSION: &str = "v1";
pub const PROCESS_PATCH_APPLY_METHOD: &str = "apply";

pub const PROCESS_TOOL_EXPOSURE_CONTRACT_VERSION: &str = "v1";
pub const PROCESS_TOOL_EXPOSURE_SELECT_METHOD: &str = "select";

pub const PROCESS_POLICY_CONTRACT_VERSION: &str = "v1";
pub const PROCESS_POLICY_EVALUATE_METHOD: &str = "evaluate";
pub const PROCESS_POLICY_VISIBILITY_METHOD: &str = "evaluate_visibility";

pub const PROCESS_RENDERER_CONTRACT_VERSION: &str = "v1";
pub const PROCESS_RENDERER_RENDER_METHOD: &str = "render";

pub const PROCESS_CONTEXT_CONTRACT_VERSION: &str = "v1";
pub const PROCESS_CONTEXT_BUILD_METHOD: &str = "build";
pub const CONTEXT_HOST_SEARCH_METHOD: &str = "host.search.query";
pub const CONTEXT_HOST_RECALL_MEMORY_METHOD: &str = "host.memory.recall";
pub const CONTEXT_HOST_PROVIDER_METHOD: &str = "host.context.provide";

pub const PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION: &str = "v1";
pub const PROCESS_CONTEXT_PROVIDER_METHOD: &str = "provide";

pub const PROCESS_TOOL_CONTRACT_VERSION: &str = "v1";
pub const PROCESS_TOOL_LIST_METHOD: &str = "list";
pub const PROCESS_TOOL_INVOKE_METHOD: &str = "invoke";

/// Strict response envelope shared by the simple process slots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessModuleResponse<T> {
    pub result: T,
}

impl<T> ProcessModuleResponse<T> {
    pub fn new(result: T) -> Self {
        Self { result }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessMemoryRememberInput {
    pub item: MemoryItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessMemoryRecallInput {
    pub query: MemoryQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessPatchInput {
    pub patch: Patch,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessToolExposureInput {
    pub input: ToolExposureInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessPolicyEvaluateInput {
    pub call: ToolCall,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_spec: Option<ToolSpec>,
    #[serde(default)]
    pub granted_permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessPolicyVisibilityInput {
    pub cwd: PathBuf,
    pub tool_spec: ToolSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessRendererInput {
    pub output: AgentOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessContextInput {
    pub task: AgentTask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessContextSearchInput {
    pub query: super::SearchQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessContextRecallInput {
    pub query: MemoryQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessContextProviderInput {
    pub provider_id: String,
    pub task: AgentTask,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessToolInvokeInput {
    pub call: ToolCall,
    pub cwd: PathBuf,
    pub owner: ToolInvocationOwner,
}

pub type ProcessMemoryRememberResponse = ProcessModuleResponse<()>;
pub type ProcessMemoryRecallResponse = ProcessModuleResponse<Vec<MemoryItem>>;
pub type ProcessPatchResponse = ProcessModuleResponse<PatchResult>;
pub type ProcessToolExposureResponse = ProcessModuleResponse<super::ToolExposureOutput>;
pub type ProcessPolicyResponse = ProcessModuleResponse<PolicyDecision>;
pub type ProcessRendererResponse = ProcessModuleResponse<String>;
pub type ProcessContextResponse = ProcessModuleResponse<ContextBundle>;
pub type ProcessContextChunksResponse = ProcessModuleResponse<Vec<ContextChunk>>;
pub type ProcessToolListResponse = ProcessModuleResponse<Vec<ToolSpec>>;
pub type ProcessToolInvokeResponse = ProcessModuleResponse<ToolResult>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_envelope_rejects_old_bare_values_and_unknown_fields() {
        serde_json::from_value::<ProcessRendererResponse>(serde_json::json!("hello"))
            .expect_err("bare result must be rejected");
        serde_json::from_value::<ProcessRendererResponse>(serde_json::json!({
            "result": "hello",
            "legacy": true
        }))
        .expect_err("unknown response fields must be rejected");
    }
}
