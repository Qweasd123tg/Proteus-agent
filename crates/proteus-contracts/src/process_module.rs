//! Rust helper API used by the bundled process-module worker.
//!
//! This is not an in-process extension ABI. Implementations linked into one
//! executable use ordinary Rust trait objects; the only host boundary is the
//! versioned process protocol from [`crate::contracts`]. Out-of-tree workers
//! may implement that JSON protocol directly in any language and do not need
//! this helper module.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    contracts::ExecutionAttribution,
    domain::{
        AgentOutput, AgentTask, HistoryCompactionReport, ModelRef, ReasoningConfig, SessionId,
        ThreadId, TurnId,
    },
    model_standard::{CanonicalMessage, InstructionBlock},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessModuleError {
    pub message: String,
}

impl ProcessModuleError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProcessModuleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProcessModuleError {}

pub type ProcessModuleResult<T> = Result<T, ProcessModuleError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolModuleInvocationContext {
    pub cwd: PathBuf,
    pub attribution: ExecutionAttribution,
    #[serde(default)]
    pub config: serde_json::Value,
}

pub trait ToolModuleHost: Send + Sync {
    fn is_cancelled(&self) -> ProcessModuleResult<bool>;
}

pub type ToolModuleHostMut<'a> = dyn ToolModuleHost + 'a;

pub trait ToolModule: Send + Sync + 'static {
    fn spec_json(&self) -> String;

    fn invoke_json(
        &self,
        call_json: String,
        context_json: String,
        host: &mut dyn ToolModuleHost,
    ) -> ProcessModuleResult<String>;
}

pub type ToolModuleObject = Box<dyn ToolModule>;

pub trait PolicyModule: Send + Sync + 'static {
    fn evaluate_json(&self, call_json: String, context_json: String)
    -> ProcessModuleResult<String>;

    fn evaluate_visibility_json(&self, context_json: String) -> ProcessModuleResult<String>;
}

pub type PolicyModuleObject = Box<dyn PolicyModule>;

pub trait PatchModule: Send + Sync + 'static {
    fn apply_json(&self, patch_json: String, cwd: String) -> ProcessModuleResult<String>;
}

pub type PatchModuleObject = Box<dyn PatchModule>;

pub trait SearchModule: Send + Sync + 'static {
    fn search_json(&self, query_json: String) -> ProcessModuleResult<String>;
}

pub type SearchModuleObject = Box<dyn SearchModule>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MemoryModuleInvocationContext {
    pub attribution: ExecutionAttribution,
    #[serde(default)]
    pub config: serde_json::Value,
}

pub trait MemoryModuleHost: Send + Sync {
    fn is_cancelled(&self) -> ProcessModuleResult<bool>;
}

pub type MemoryModuleHostMut<'a> = dyn MemoryModuleHost + 'a;

pub trait MemoryModule: Send + Sync + 'static {
    fn remember_json(
        &self,
        item_json: String,
        context_json: String,
        host: &mut dyn MemoryModuleHost,
    ) -> ProcessModuleResult<()>;
    fn recall_json(
        &self,
        query_json: String,
        context_json: String,
        host: &mut dyn MemoryModuleHost,
    ) -> ProcessModuleResult<String>;
}

pub type MemoryModuleObject = Box<dyn MemoryModule>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ContextProviderModuleInput {
    pub provider_id: String,
    pub task: AgentTask,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

pub trait ContextProviderModule: Send + Sync + 'static {
    fn provide_json(&self, input_json: String) -> ProcessModuleResult<String>;
}

pub type ContextProviderModuleObject = Box<dyn ContextProviderModule>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ContextBuilderModuleInput {
    pub task: AgentTask,
    #[serde(default)]
    pub config: serde_json::Value,
}

pub trait ContextBuilderModuleHost: Send + Sync {
    fn search_json(&self, query_json: String) -> ProcessModuleResult<String>;
    fn recall_memory_json(&self, query_json: String) -> ProcessModuleResult<String>;
    fn context_provider_json(
        &self,
        provider_id: String,
        input_json: String,
    ) -> ProcessModuleResult<String>;
}

pub type ContextBuilderModuleHostMut<'a> = dyn ContextBuilderModuleHost + 'a;

pub trait ContextBuilderModule: Send + Sync + 'static {
    fn build_json(
        &self,
        input_json: String,
        host: &mut dyn ContextBuilderModuleHost,
    ) -> ProcessModuleResult<String>;
}

pub type ContextBuilderModuleObject = Box<dyn ContextBuilderModule>;

pub trait CompactorModuleHost: Send + Sync {
    fn is_cancelled(&self) -> ProcessModuleResult<bool>;
    fn complete_model_json(&self, request_json: String) -> ProcessModuleResult<String>;
}

pub type CompactorModuleHostMut<'a> = dyn CompactorModuleHost + 'a;

pub trait CompactorModule: Send + Sync + 'static {
    fn compact_json(
        &self,
        input_json: String,
        host: &mut dyn CompactorModuleHost,
    ) -> ProcessModuleResult<String>;
}

pub type CompactorModuleObject = Box<dyn CompactorModule>;

pub trait ToolExposureModule: Send + Sync + 'static {
    fn select_json(&self, input_json: String) -> ProcessModuleResult<String>;
}

pub type ToolExposureModuleObject = Box<dyn ToolExposureModule>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowModuleInput {
    pub task: AgentTask,
    #[serde(default)]
    pub history: Vec<CanonicalMessage>,
    #[serde(default)]
    pub config: serde_json::Value,
    pub runtime: WorkflowModuleRuntimeInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowModuleRuntimeInfo {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub model_ref: ModelRef,
    #[serde(default)]
    pub instructions: Vec<InstructionBlock>,
    #[serde(default)]
    pub reasoning: ReasoningConfig,
    #[serde(default)]
    pub max_input_tokens: Option<u32>,
    pub model_timeout_ms: u64,
    pub context_timeout_ms: u64,
    pub workflow_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowModuleOutput {
    pub output: AgentOutput,
    #[serde(default)]
    pub new_messages: Vec<CanonicalMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_replacement: Option<Vec<CanonicalMessage>>,
    #[serde(default)]
    pub compactions: Vec<HistoryCompactionReport>,
}

pub trait WorkflowModuleHost: Send + Sync {
    fn is_cancelled(&self) -> ProcessModuleResult<bool>;
    fn queued_user_messages(&self) -> ProcessModuleResult<u32>;
    fn build_context_json(&self, task_json: String) -> ProcessModuleResult<String>;
    fn complete_model_json(&self, request_json: String) -> ProcessModuleResult<String>;
    fn compact_history_json(&self, input_json: String) -> ProcessModuleResult<String>;
    fn visible_tools_json(&self, cwd: String) -> ProcessModuleResult<String>;
    fn select_tools_json(&self, request_json: String) -> ProcessModuleResult<String>;
    fn execute_tool_json(
        &self,
        task_json: String,
        call_json: String,
    ) -> ProcessModuleResult<String>;
    fn emit_event_json(&self, event_json: String) -> ProcessModuleResult<()>;
    fn execute_tools_json(
        &self,
        task_json: String,
        calls_json: String,
    ) -> ProcessModuleResult<String>;
}

pub type WorkflowModuleHostMut<'a> = dyn WorkflowModuleHost + 'a;

pub trait WorkflowModule: Send + Sync + 'static {
    fn run_json(
        &self,
        input_json: String,
        host: &mut dyn WorkflowModuleHost,
    ) -> ProcessModuleResult<String>;
}

pub type WorkflowModuleObject = Box<dyn WorkflowModule>;

/// Link-time registry used only to assemble one process worker executable.
/// It does not cross the host boundary and grants no runtime capabilities.
pub trait ModuleRegistry {
    /// Opaque config received in the process initialize handshake.
    fn module_config(&self) -> &serde_json::Value;

    fn register_tool(&mut self, tool: ToolModuleObject) -> ProcessModuleResult<()>;
    fn register_policy(
        &mut self,
        module_id: String,
        policy: PolicyModuleObject,
    ) -> ProcessModuleResult<()>;
    fn register_patch(
        &mut self,
        module_id: String,
        applier: PatchModuleObject,
    ) -> ProcessModuleResult<()>;
    fn register_search(
        &mut self,
        module_id: String,
        backend: SearchModuleObject,
    ) -> ProcessModuleResult<()>;
    fn register_memory(
        &mut self,
        module_id: String,
        store: MemoryModuleObject,
    ) -> ProcessModuleResult<()>;
    fn register_context_provider(
        &mut self,
        provider_id: String,
        provider: ContextProviderModuleObject,
    ) -> ProcessModuleResult<()>;
    fn register_context(
        &mut self,
        module_id: String,
        builder: ContextBuilderModuleObject,
    ) -> ProcessModuleResult<()>;
    fn register_compactor(
        &mut self,
        module_id: String,
        compactor: CompactorModuleObject,
    ) -> ProcessModuleResult<()>;
    fn register_tool_exposure(
        &mut self,
        module_id: String,
        exposure: ToolExposureModuleObject,
    ) -> ProcessModuleResult<()>;
    fn register_workflow(
        &mut self,
        module_id: String,
        workflow: WorkflowModuleObject,
    ) -> ProcessModuleResult<()>;
}
