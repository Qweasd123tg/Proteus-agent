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

    // Будущие методы: register_tool, register_search_backend, register_context_builder,
    // register_memory_store, register_memory_policy, register_approval_policy,
    // register_patch_applier. Добавляются как prefix-fields (sabi-совместимо).
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
