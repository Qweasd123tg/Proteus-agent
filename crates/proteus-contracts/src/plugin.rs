#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
//! Plugin API: структуры и trait'ы для dylib-плагинов.
//!
//! Плагин — отдельный Cargo project, который собирается в `cdylib`
//! и экспортирует `PluginRoot` через `#[export_root_module]`.
//! Ядро загружает плагин через `PluginRoot_Ref::load_from_file` и вызывает
//! `register_modules`, чтобы плагин зарегистрировал свои реализации в
//! Registry.
//!
//! ## Архитектура
//!
//! Плагин видит **абстрактный** `RegistryInterface` (sabi_trait) — он не
//! знает деталей внутренней HashMap ядра. Через этот интерфейс плагин вызывает
//! методы `register_renderer`, `register_tool` и т.п., передавая в них свои
//! sabi_trait объекты (например, `RendererObject`).
//!
//! Ядро в своей реализации `RegistryInterface` связывает `(slot, module_id)`
//! с фабрикой, которая вернёт этот sabi-объект.
//!
//! ## ABI compatibility
//!
//! abi_stable при загрузке плагина сверяет:
//! - Версию `abi_stable` (совместимость макросов/типов).
//! - Layout `PluginRoot` и всех referenced типов (fields, vtables).
//! - Версию плагина (в `VERSION_STRINGS`).
//!
//! Если что-то несовместимо — загрузка возвращает `LibraryError`, плагин
//! не загружается, ядро продолжает работать без него.

use abi_stable::{
    StableAbi, declare_root_module_statics,
    library::RootModule,
    package_version_strings, sabi_trait,
    sabi_types::VersionStrings,
    std_types::{RResult, RStr, RString},
};

use serde::{Deserialize, Serialize};

use crate::{
    contracts::RendererObject,
    domain::{
        AgentOutput, AgentTask, HistoryCompactionReport, ModelRef, ReasoningConfig, SessionId,
        ThreadId, TurnId,
    },
    model_standard::CanonicalMessage,
};

/// Sync sabi_trait для tool-плагинов.
///
/// Builtin tools в ядре остаются async (используют `tokio::fs`, `tokio::process`).
/// Плагины же реализуют sync-версию, которая внутри может создавать свой
/// локальный tokio runtime или использовать blocking I/O (`reqwest::blocking`,
/// `std::fs`, `std::process::Command`).
///
/// Ядро оборачивает `PluginTool` в обычный `Tool` через `spawn_blocking`, так
/// что concurrency не страдает.
///
/// ## DTO через границу
///
/// `ToolCall` и `ToolResult` сериализуются в JSON (`RString`) для передачи
/// через FFI. Плагин десериализует через `serde_json` обратно в native DTO.
/// Это избавляет от необходимости переделывать DTO в `#[repr(C)]` (у них есть
/// `serde_json::Value`-поля, которые не прямо перекладываются в FFI-safe).
///
/// ## ToolSpec
///
/// `spec()` возвращает JSON с описанием tool. Ядро десериализует в `ToolSpec`
/// и регистрирует в ToolRegistry.
#[sabi_trait]
pub trait PluginTool: Send + Sync + 'static {
    /// Возвращает JSON-сериализованный `ToolSpec`.
    fn spec_json(&self) -> RString;

    /// Вызывает tool. `call_json` — сериализованный `ToolCall`.
    /// `cwd` — рабочая директория для tool'а.
    /// Возврат — сериализованный `ToolResult` или ошибка.
    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError>;
}

/// Ошибка выполнения tool-плагина.
#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginToolError {
    pub message: RString,
}

impl PluginToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginToolError {}

/// Ffi-safe trait object для PluginTool.
pub type PluginToolObject = PluginTool_TO<abi_stable::std_types::RBox<()>>;

/// Sync sabi_trait для approval-policy плагинов.
///
/// Ядро-trait `ApprovalPolicy` уже sync — маппинг 1:1, без spawn_blocking.
/// DTO передаются через FFI как JSON (`RString`), аналогично `PluginTool`.
///
/// ## JSON-форма
///
/// - `call_json` — сериализованный `ToolCall`.
/// - `ctx_json` для `evaluate_json` — `PluginPolicyContextDto` (см. ниже).
/// - `ctx_json` для `evaluate_visibility_json` — `PluginPolicyVisibilityContextDto`.
/// - Возврат — сериализованный `PolicyDecision`.
#[sabi_trait]
pub trait PluginApprovalPolicy: Send + Sync + 'static {
    fn evaluate_json(
        &self,
        call_json: RString,
        ctx_json: RString,
    ) -> RResult<RString, PluginPolicyError>;

    fn evaluate_visibility_json(&self, ctx_json: RString) -> RResult<RString, PluginPolicyError>;
}

/// Ошибка выполнения approval-policy плагина.
#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginPolicyError {
    pub message: RString,
}

impl PluginPolicyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginPolicyError {}

/// Ffi-safe trait object для PluginApprovalPolicy.
pub type PolicyObject = PluginApprovalPolicy_TO<abi_stable::std_types::RBox<()>>;

/// Sync sabi_trait для patch-applier плагинов.
///
/// Ядро-trait `PatchApplier` async, поэтому адаптер в ядре оборачивает
/// sync-вызов плагина в `spawn_blocking`. DTO через JSON.
///
/// ## JSON-форма
///
/// - `patch_json` — сериализованный `Patch` (только поле `content: String`).
/// - `cwd` — рабочая директория.
/// - Возврат — сериализованный `PatchResult`.
#[sabi_trait]
pub trait PluginPatchApplier: Send + Sync + 'static {
    fn apply_json(&self, patch_json: RString, cwd: RString) -> RResult<RString, PluginPatchError>;
}

/// Ошибка выполнения patch-applier плагина.
#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginPatchError {
    pub message: RString,
}

impl PluginPatchError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginPatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginPatchError {}

/// Ffi-safe trait object для PluginPatchApplier.
pub type PatchApplierObject = PluginPatchApplier_TO<abi_stable::std_types::RBox<()>>;

/// Sync sabi_trait для search-backend плагинов.
///
/// Ядро-trait `SearchBackend` async, поэтому адаптер в ядре оборачивает
/// sync-вызов плагина в `spawn_blocking`. DTO через JSON.
///
/// ## JSON-форма
///
/// - `query_json` — serialized `SearchQuery`; unknown/defaulted fields must be
///   ignored by plugin implementations.
/// - Возврат — JSON массив `ContextChunk`.
#[sabi_trait]
pub trait PluginSearchBackend: Send + Sync + 'static {
    fn search_json(&self, query_json: RString) -> RResult<RString, PluginSearchError>;
}

/// Ошибка выполнения search-backend плагина.
#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginSearchError {
    pub message: RString,
}

impl PluginSearchError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginSearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginSearchError {}

/// Ffi-safe trait object для PluginSearchBackend.
pub type SearchBackendObject = PluginSearchBackend_TO<abi_stable::std_types::RBox<()>>;

/// Sync sabi_trait для memory-store плагинов.
///
/// Ядро-trait `MemoryStore` async, поэтому адаптер в ядре оборачивает
/// sync-вызов плагина в `spawn_blocking`. DTO через JSON.
///
/// ## JSON-форма
///
/// - `item_json` для `remember_json` — сериализованный `MemoryItem
///   { kind, content, metadata }`. Возврат — пустой при успехе.
/// - `query_json` для `recall_json` — сериализованный `MemoryQuery
///   { text, limit }`. Возврат — JSON массив `MemoryItem`.
#[sabi_trait]
pub trait PluginMemoryStore: Send + Sync + 'static {
    fn remember_json(&self, item_json: RString) -> RResult<(), PluginMemoryError>;
    fn recall_json(&self, query_json: RString) -> RResult<RString, PluginMemoryError>;
}

/// Ошибка выполнения memory-store плагина.
#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginMemoryError {
    pub message: RString,
}

impl PluginMemoryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginMemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginMemoryError {}

/// Ffi-safe trait object для PluginMemoryStore.
pub type MemoryStoreObject = PluginMemoryStore_TO<abi_stable::std_types::RBox<()>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginContextProviderInput {
    pub provider_id: String,
    pub task: AgentTask,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Sync sabi_trait для context-provider плагинов.
///
/// Это не полный `ContextBuilder`: ядро оставляет за собой orchestration,
/// budget и порядок chunks, а плагин возвращает вклад одного provider-а.
#[sabi_trait]
pub trait PluginContextProvider: Send + Sync + 'static {
    fn provide_json(&self, input_json: RString) -> RResult<RString, PluginContextError>;
}

#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginContextError {
    pub message: RString,
}

impl PluginContextError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginContextError {}

pub type ContextProviderObject = PluginContextProvider_TO<abi_stable::std_types::RBox<()>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginContextBuilderInput {
    pub task: AgentTask,
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Host capabilities exposed to full ContextBuilder plugins.
#[sabi_trait]
pub trait PluginContextBuilderHost: Send + Sync {
    /// Input JSON: `SearchQuery`. Output JSON: `Vec<ContextChunk>`.
    fn search_json(&self, query_json: RString) -> RResult<RString, PluginContextBuilderHostError>;

    /// Input JSON: `MemoryQuery`. Output JSON: `Vec<MemoryItem>`.
    fn recall_memory_json(
        &self,
        query_json: RString,
    ) -> RResult<RString, PluginContextBuilderHostError>;

    /// Input JSON: `PluginContextProviderInput`. Output JSON:
    /// `Vec<ContextChunk>` from an already registered provider.
    fn context_provider_json(
        &self,
        provider_id: RString,
        input_json: RString,
    ) -> RResult<RString, PluginContextBuilderHostError>;
}

#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginContextBuilderHostError {
    pub message: RString,
}

impl PluginContextBuilderHostError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginContextBuilderHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginContextBuilderHostError {}

pub type PluginContextBuilderHostMut<'a> =
    PluginContextBuilderHost_TO<'a, abi_stable::sabi_types::RMut<'a, ()>>;

/// Sync sabi_trait for full ContextBuilder plugins.
#[sabi_trait]
pub trait PluginContextBuilder: Send + Sync + 'static {
    /// Input JSON: `PluginContextBuilderInput`. Output JSON: `ContextBundle`.
    fn build_json(
        &self,
        input_json: RString,
        host: &mut PluginContextBuilderHostMut<'_>,
    ) -> RResult<RString, PluginContextError>;
}

pub type ContextBuilderObject = PluginContextBuilder_TO<abi_stable::std_types::RBox<()>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMemoryPolicyInput {
    pub task: AgentTask,
    pub output: AgentOutput,
    #[serde(default)]
    pub new_messages: Vec<CanonicalMessage>,
}

/// Sync sabi_trait для memory-policy плагинов.
///
/// Плагин возвращает декларативный `MemoryPolicyPlan`. Ядро валидирует и
/// применяет операции к активному `MemoryStore`, поэтому plugin не получает
/// mutable handle к памяти.
#[sabi_trait]
pub trait PluginMemoryPolicy: Send + Sync + 'static {
    fn after_turn_json(&self, input_json: RString) -> RResult<RString, PluginMemoryPolicyError>;
}

#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginMemoryPolicyError {
    pub message: RString,
}

impl PluginMemoryPolicyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginMemoryPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginMemoryPolicyError {}

pub type MemoryPolicyObject = PluginMemoryPolicy_TO<abi_stable::std_types::RBox<()>>;

/// Sync ABI for request-time history compaction plugins.
///
/// This slot returns model-facing messages only. Core does not rewrite the
/// durable session log through this contract.
#[sabi_trait]
pub trait PluginHistoryCompactor: Send + Sync + 'static {
    fn compact_json(
        &self,
        input_json: RString,
        host: &mut PluginCompactorHostMut<'_>,
    ) -> RResult<RString, PluginCompactionError>;
}

/// Host capabilities exposed to compactor plugins.
///
/// A compactor may ask the runtime model to summarize history, but it does not
/// receive tool, memory, policy, or session mutation capabilities.
#[sabi_trait]
pub trait PluginCompactorHost: Send + Sync {
    fn is_cancelled(&self) -> RResult<bool, PluginCompactionError>;

    /// Input JSON: `CanonicalModelRequest`. Output JSON:
    /// `CanonicalModelResponse`.
    fn complete_model_json(&self, request_json: RString)
    -> RResult<RString, PluginCompactionError>;
}

#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginCompactionError {
    pub message: RString,
}

impl PluginCompactionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginCompactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginCompactionError {}

pub type CompactorObject = PluginHistoryCompactor_TO<abi_stable::std_types::RBox<()>>;
pub type PluginCompactorHostMut<'a> =
    PluginCompactorHost_TO<'a, abi_stable::sabi_types::RMut<'a, ()>>;

/// Sync ABI for tool exposure/search plugins.
///
/// Core computes policy-visible candidate tools first. This plugin selects the
/// subset that should be exposed to a model request.
#[sabi_trait]
pub trait PluginToolExposure: Send + Sync + 'static {
    fn select_json(&self, input_json: RString) -> RResult<RString, PluginToolExposureError>;
}

#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginToolExposureError {
    pub message: RString,
}

impl PluginToolExposureError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginToolExposureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginToolExposureError {}

pub type ToolExposureObject = PluginToolExposure_TO<abi_stable::std_types::RBox<()>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginWorkflowInput {
    pub task: AgentTask,
    #[serde(default)]
    pub history: Vec<CanonicalMessage>,
    pub runtime: PluginWorkflowRuntimeInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginWorkflowRuntimeInfo {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub model_ref: ModelRef,
    #[serde(default)]
    pub reasoning: ReasoningConfig,
    #[serde(default)]
    pub max_input_tokens: Option<u32>,
    pub model_timeout_ms: u64,
    pub context_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginWorkflowOutput {
    pub output: AgentOutput,
    #[serde(default)]
    pub messages: Vec<CanonicalMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_messages_start: Option<usize>,
    #[serde(default)]
    pub compactions: Vec<HistoryCompactionReport>,
}

/// Host capabilities exposed to workflow plugins.
///
/// A workflow plugin should not receive `RuntimeContext` or concrete core
/// objects. Instead it calls this narrow host API. Every payload is JSON so the
/// ABI does not depend on Rust layout of complex DTOs.
#[sabi_trait]
pub trait PluginWorkflowHost: Send + Sync {
    /// Cooperative cancellation signal for long sync workflow loops.
    fn is_cancelled(&self) -> RResult<bool, PluginWorkflowHostError>;

    /// Input JSON: `AgentTask`. Output JSON: `ContextBundle`.
    fn build_context_json(&self, task_json: RString) -> RResult<RString, PluginWorkflowHostError>;

    /// Input JSON: `CanonicalModelRequest`. Output JSON:
    /// `CanonicalModelResponse`.
    fn complete_model_json(
        &self,
        request_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError>;

    /// Input JSON: `CompactionInput`. Output JSON: `CompactionOutput`.
    fn compact_history_json(
        &self,
        input_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError>;

    /// Input: cwd string. Output JSON: `Vec<ToolSpec>` after visibility policy.
    fn visible_tools_json(&self, cwd: RString) -> RResult<RString, PluginWorkflowHostError>;

    /// Input JSON: `ToolExposureRequest`. Output JSON: `ToolExposureOutput`.
    fn select_tools_json(&self, request_json: RString)
    -> RResult<RString, PluginWorkflowHostError>;

    /// Input JSON: `AgentTask` and `ToolCall`. Output JSON: `ToolResult`.
    fn execute_tool_json(
        &self,
        task_json: RString,
        call_json: RString,
    ) -> RResult<RString, PluginWorkflowHostError>;

    /// Input JSON: `Event`. Emits with current runtime event context.
    fn emit_event_json(&self, event_json: RString) -> RResult<(), PluginWorkflowHostError>;
}

#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginWorkflowHostError {
    pub message: RString,
}

impl PluginWorkflowHostError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginWorkflowHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginWorkflowHostError {}

pub type PluginWorkflowHostMut<'a> =
    PluginWorkflowHost_TO<'a, abi_stable::sabi_types::RMut<'a, ()>>;

/// Sync sabi_trait for workflow plugins.
///
/// The plugin runs synchronously; the core adapter executes it in
/// `spawn_blocking`. Async operations go through `PluginWorkflowHost`, which
/// bridges back into the runtime.
#[sabi_trait]
pub trait PluginWorkflow: Send + Sync + 'static {
    /// Input JSON: `PluginWorkflowInput`. Output JSON: `PluginWorkflowOutput`.
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError>;
}

#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginWorkflowError {
    pub message: RString,
}

impl PluginWorkflowError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginWorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginWorkflowError {}

pub type WorkflowObject = PluginWorkflow_TO<abi_stable::std_types::RBox<()>>;

/// Ошибка регистрации модуля плагином.
#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct PluginRegisterError {
    pub message: RString,
}

impl PluginRegisterError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for PluginRegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for PluginRegisterError {}

/// Интерфейс, который ядро передаёт плагину для регистрации модулей.
///
/// Плагин вызывает `register_renderer(module_id, renderer)` и аналоги.
/// Ядро в своей реализации добавляет запись в Registry.
#[sabi_trait]
pub trait PluginRegistry: Send + Sync {
    /// Регистрирует Renderer под указанным module_id в slot `renderer`.
    fn register_renderer(
        &mut self,
        module_id: RString,
        renderer: RendererObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует Tool от плагина. Внутри плагина tool реализует
    /// sync-версию `PluginTool` (поскольку sabi_trait не поддерживает async).
    /// Ядро оборачивает его в обычный async `Tool` через spawn_blocking.
    fn register_tool(&mut self, tool: PluginToolObject) -> RResult<(), PluginRegisterError>;

    /// Регистрирует ApprovalPolicy под module_id в slot `policy`.
    /// Ядро-trait `ApprovalPolicy` sync, маппинг прямой.
    fn register_approval_policy(
        &mut self,
        module_id: RString,
        policy: PolicyObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует PatchApplier под module_id в slot `patch`.
    /// Ядро-trait `PatchApplier` async — адаптер в ядре мостит через
    /// spawn_blocking.
    fn register_patch_applier(
        &mut self,
        module_id: RString,
        applier: PatchApplierObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует SearchBackend под module_id в slot `search`.
    /// Ядро-trait `SearchBackend` async — адаптер в ядре мостит через
    /// spawn_blocking.
    fn register_search_backend(
        &mut self,
        module_id: RString,
        backend: SearchBackendObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует MemoryStore под module_id в slot `memory`.
    /// Ядро-trait `MemoryStore` async — адаптер в ядре мостит через
    /// spawn_blocking.
    fn register_memory_store(
        &mut self,
        module_id: RString,
        store: MemoryStoreObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует внешний provider для context builders, которые поддерживают
    /// provider pipeline.
    fn register_context_provider(
        &mut self,
        provider_id: RString,
        provider: ContextProviderObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует полный ContextBuilder под module_id в slot `context`.
    fn register_context_builder(
        &mut self,
        module_id: RString,
        builder: ContextBuilderObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует declarative MemoryPolicy под module_id в slot `memory_policy`.
    fn register_memory_policy(
        &mut self,
        module_id: RString,
        policy: MemoryPolicyObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует request-time HistoryCompactor под module_id в slot `compactor`.
    fn register_compactor(
        &mut self,
        module_id: RString,
        compactor: CompactorObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует ToolExposure под module_id в slot `tool_exposure`.
    fn register_tool_exposure(
        &mut self,
        module_id: RString,
        exposure: ToolExposureObject,
    ) -> RResult<(), PluginRegisterError>;

    /// Регистрирует Workflow под module_id в slot `workflow`.
    fn register_workflow(
        &mut self,
        module_id: RString,
        workflow: WorkflowObject,
    ) -> RResult<(), PluginRegisterError>;
}

/// Тип trait-объекта PluginRegistry, передаваемого в плагин.
pub type PluginRegistryMut<'a> = PluginRegistry_TO<'a, abi_stable::sabi_types::RMut<'a, ()>>;

/// Root module плагина — то, что плагин экспортирует через
/// `#[export_root_module]`, и что ядро загружает через `PluginRoot_Ref::load_from_file`.
///
/// В текущем workspace старые собранные `.so` не считаются совместимыми между
/// refactor-итерациями: новые plugin-facing slots добавляются прямо в
/// `PluginRegistry`, а плагины пересобираются вместе с `proteus-contracts`.
#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = PluginRoot_Ref, prefix_fields = PluginRoot_Prefix)))]
pub struct PluginRoot {
    /// Название плагина для логов. Не обязан совпадать с именем файла.
    /// Используется `RStr<'static>` — плагин передаёт строковый литерал.
    pub name: RStr<'static>,

    /// Описание плагина — свободный текст.
    pub description: RStr<'static>,

    /// Регистрирует все модули плагина в переданном Registry.
    ///
    /// Вызывается ядром один раз сразу после успешной загрузки плагина.
    /// Плагин внутри этой функции должен вызвать register_renderer / etc.
    #[sabi(last_prefix_field)]
    pub register_modules:
        extern "C" fn(&mut PluginRegistryMut<'_>) -> RResult<(), PluginRegisterError>,
}

impl RootModule for PluginRoot_Ref {
    const BASE_NAME: &'static str = "proteus_plugin";
    const NAME: &'static str = "proteus_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();

    declare_root_module_statics! {PluginRoot_Ref}
}

/// Re-export макросов abi_stable для плагинов.
///
/// Плагин использует их так:
/// ```ignore
/// use proteus_contracts::plugin::{PluginRoot, PluginRootBuilder};
/// use proteus_contracts::abi_stable::prefix_type::PrefixTypeTrait;
/// use proteus_contracts::abi_stable::export_root_module;
///
/// #[export_root_module]
/// pub fn get_plugin_root() -> PluginRoot_Ref {
///     PluginRoot {
///         name: "my-plugin".into(),
///         description: "does something".into(),
///         register_modules,
///     }.leak_into_prefix()
/// }
/// ```
pub use abi_stable::export_root_module;
