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
    StableAbi,
    declare_root_module_statics,
    library::RootModule,
    package_version_strings,
    sabi_trait,
    sabi_types::VersionStrings,
    std_types::{RResult, RStr, RString},
};

use crate::contracts::RendererObject;

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
    fn invoke_json(
        &self,
        call_json: RString,
        cwd: RString,
    ) -> RResult<RString, PluginToolError>;
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

    fn evaluate_visibility_json(
        &self,
        ctx_json: RString,
    ) -> RResult<RString, PluginPolicyError>;
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
    fn apply_json(
        &self,
        patch_json: RString,
        cwd: RString,
    ) -> RResult<RString, PluginPatchError>;
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
    fn register_tool(
        &mut self,
        tool: PluginToolObject,
    ) -> RResult<(), PluginRegisterError>;

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

    // Будущие: register_search_backend, register_context_builder,
    // register_memory_store, register_memory_policy.
}

/// Тип trait-объекта PluginRegistry, передаваемого в плагин.
pub type PluginRegistryMut<'a> =
    PluginRegistry_TO<'a, abi_stable::sabi_types::RMut<'a, ()>>;

/// Root module плагина — то, что плагин экспортирует через
/// `#[export_root_module]`, и что ядро загружает через `PluginRoot_Ref::load_from_file`.
///
/// Структура префиксная (prefix type) — это позволяет добавлять новые поля
/// в конце без breaking change. Старые плагины, собранные против более ранней
/// версии, продолжают загружаться; новые поля для них будут недоступны,
/// но это не крашит load.
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
    pub register_modules: extern "C" fn(&mut PluginRegistryMut<'_>) -> RResult<(), PluginRegisterError>,
}

impl RootModule for PluginRoot_Ref {
    const BASE_NAME: &'static str = "agent_plugin";
    const NAME: &'static str = "agent_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();

    declare_root_module_statics! {PluginRoot_Ref}
}

/// Re-export макросов abi_stable для плагинов.
///
/// Плагин использует их так:
/// ```ignore
/// use agent_contracts::plugin::{PluginRoot, PluginRootBuilder};
/// use agent_contracts::abi_stable::prefix_type::PrefixTypeTrait;
/// use agent_contracts::abi_stable::export_root_module;
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
