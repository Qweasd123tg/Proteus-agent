//! Strict process DTOs for slots whose canonical Rust traits do not carry
//! their own wire representation.
//!
//! The host chooses the contract from the slot, never from `module_id`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    contracts::ExecutionAttribution,
    domain::{
        AgentTask, ContextBundle, ContextChunk, MemoryItem, MemoryQuery, PolicyDecision, ToolCall,
        ToolResult, ToolSpec,
    },
};

use super::ToolExposureInput;

pub const PROCESS_MEMORY_CONTRACT_VERSION: &str = "v2";
pub const PROCESS_MEMORY_REMEMBER_METHOD: &str = "remember";
pub const PROCESS_MEMORY_RECALL_METHOD: &str = "recall";

pub const PROCESS_TOOL_EXPOSURE_CONTRACT_VERSION: &str = "v4";
pub const PROCESS_TOOL_EXPOSURE_SELECT_METHOD: &str = "select";

pub const PROCESS_POLICY_CONTRACT_VERSION: &str = "v2";
pub const PROCESS_POLICY_EVALUATE_METHOD: &str = "evaluate";
pub const PROCESS_POLICY_VISIBILITY_METHOD: &str = "evaluate_visibility";

pub const PROCESS_CONTEXT_CONTRACT_VERSION: &str = "v2";
pub const PROCESS_CONTEXT_BUILD_METHOD: &str = "build";
pub const CONTEXT_HOST_SEARCH_METHOD: &str = "host.search.query";
pub const CONTEXT_HOST_RECALL_MEMORY_METHOD: &str = "host.memory.recall";
pub const CONTEXT_HOST_PROVIDER_METHOD: &str = "host.context.provide";

pub const PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION: &str = "v3";
pub const PROCESS_CONTEXT_PROVIDER_METHOD: &str = "provide";
pub const PROCESS_CONTEXT_PROVIDER_CATALOG_METHOD: &str = "catalog";

pub const PROCESS_TOOL_CONTRACT_VERSION: &str = "v5";
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
    pub attribution: ExecutionAttribution,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessMemoryRecallInput {
    pub query: MemoryQuery,
    pub attribution: ExecutionAttribution,
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
    pub attribution: ExecutionAttribution,
    pub skills: crate::domain::SkillRuntimeSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessContextProviderRequest {
    pub input: ProcessContextProviderInput,
    pub skills: crate::domain::SkillRuntimeSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSkillCatalogInput {
    pub cwd: PathBuf,
    pub skills: crate::domain::SkillRuntimeSettings,
}

pub type ProcessSkillCatalogResponse = ProcessModuleResponse<Option<crate::domain::SkillCatalog>>;

pub type ProcessMemoryRememberResponse = ProcessModuleResponse<()>;
pub type ProcessMemoryRecallResponse = ProcessModuleResponse<Vec<MemoryItem>>;
pub type ProcessToolExposureResponse = ProcessModuleResponse<super::ToolExposureOutput>;
pub type ProcessPolicyResponse = ProcessModuleResponse<PolicyDecision>;
pub type ProcessContextResponse = ProcessModuleResponse<ContextBundle>;
pub type ProcessContextChunksResponse = ProcessModuleResponse<Vec<ContextChunk>>;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessToolDefinition {
    pub spec: ToolSpec,
    pub model_visible: bool,
    #[serde(deserialize_with = "Option::deserialize")]
    pub user_command: Option<super::ToolUserCommand>,
}

pub type ProcessToolListResponse = ProcessModuleResponse<Vec<ProcessToolDefinition>>;
pub type ProcessToolInvokeResponse = ProcessModuleResponse<ToolResult>;

#[cfg(test)]
mod tests {
    use crate::domain::{new_call_id, new_execution_id};

    use super::*;

    #[test]
    fn response_envelope_rejects_old_bare_values_and_unknown_fields() {
        serde_json::from_value::<ProcessModuleResponse<String>>(serde_json::json!("hello"))
            .expect_err("bare result must be rejected");
        serde_json::from_value::<ProcessModuleResponse<String>>(serde_json::json!({
            "result": "hello",
            "legacy": true
        }))
        .expect_err("unknown response fields must be rejected");
        let input = super::super::ToolExposureInput::new(
            super::super::ToolExposureRequest::new(AgentTask::new(
                "probe",
                std::path::PathBuf::from("."),
            )),
            vec![],
        );
        let mut value = serde_json::to_value(ProcessToolExposureInput { input }).unwrap();
        value["input"]["unexpected_field"] = serde_json::json!({});
        serde_json::from_value::<ProcessToolExposureInput>(value)
            .expect_err("nested input must be strict");

        let definition = ProcessToolDefinition {
            spec: ToolSpec::new(
                "probe",
                "Probe",
                serde_json::json!({}),
                crate::domain::ToolSafety::ReadOnly,
            ),
            model_visible: false,
            user_command: Some(super::super::ToolUserCommand {
                name: "probe".into(),
                description: "Probe".into(),
                arguments: "[ARGS]".into(),
            }),
        };
        let value = serde_json::to_value(&definition).unwrap();
        serde_json::from_value::<ProcessToolDefinition>(value.clone()).unwrap();
        for field in ["model_visible", "user_command"] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            serde_json::from_value::<ProcessToolDefinition>(missing)
                .expect_err("tool definition fields are required");
        }
        serde_json::from_value::<ProcessToolDefinition>(
            serde_json::to_value(&definition.spec).unwrap(),
        )
        .expect_err("bare specs are not tool definitions");
        let mut unknown = value;
        unknown["user_command"]["legacy"] = serde_json::json!(true);
        serde_json::from_value::<ProcessToolDefinition>(unknown)
            .expect_err("command definitions are strict");
    }

    #[test]
    fn tool_v2_requires_execution_attribution_and_accepts_detached_execution() {
        let input = ProcessToolInvokeInput {
            call: ToolCall::new(new_call_id(), "probe", serde_json::json!({})),
            cwd: PathBuf::from("/workspace"),
            attribution: ExecutionAttribution::detached(new_execution_id()),
            skills: Default::default(),
        };
        let value = serde_json::to_value(&input).expect("tool input");
        serde_json::from_value::<ProcessToolInvokeInput>(value.clone())
            .expect("detached execution must cross tool v2");

        let mut missing_attribution = value.clone();
        missing_attribution
            .as_object_mut()
            .expect("tool object")
            .remove("attribution");
        serde_json::from_value::<ProcessToolInvokeInput>(missing_attribution)
            .expect_err("execution attribution is mandatory");

        let mut legacy_owner = value;
        let object = legacy_owner.as_object_mut().expect("tool object");
        object.remove("attribution");
        object.insert(
            "owner".to_owned(),
            serde_json::json!({
                "session_id": "session_legacy",
                "thread_id": "thread_legacy",
                "turn_id": "turn_legacy"
            }),
        );
        serde_json::from_value::<ProcessToolInvokeInput>(legacy_owner)
            .expect_err("tool v1 owner must not be accepted by v2");
    }

    #[test]
    fn memory_v2_requires_execution_attribution_and_rejects_v1_payloads() {
        let attribution = ExecutionAttribution::detached(new_execution_id());
        let remember = ProcessMemoryRememberInput {
            item: MemoryItem::new("fact", "detached memory", serde_json::Value::Null),
            attribution,
        };
        let value = serde_json::to_value(&remember).expect("memory input");
        serde_json::from_value::<ProcessMemoryRememberInput>(value.clone())
            .expect("detached execution must cross memory v2");

        let mut v1 = value.clone();
        v1.as_object_mut()
            .expect("memory object")
            .remove("attribution");
        serde_json::from_value::<ProcessMemoryRememberInput>(v1)
            .expect_err("memory v1 payload must not be accepted by v2");

        let recall = ProcessMemoryRecallInput {
            query: MemoryQuery::new("detached", 5),
            attribution,
        };
        let mut legacy_owner = serde_json::to_value(recall).expect("recall input");
        let object = legacy_owner.as_object_mut().expect("recall object");
        object.remove("attribution");
        object.insert(
            "owner".to_owned(),
            serde_json::json!({
                "session_id": "session_legacy",
                "thread_id": "thread_legacy",
                "turn_id": "turn_legacy"
            }),
        );
        serde_json::from_value::<ProcessMemoryRecallInput>(legacy_owner)
            .expect_err("memory v1 owner must not be accepted by v2");
    }
}
