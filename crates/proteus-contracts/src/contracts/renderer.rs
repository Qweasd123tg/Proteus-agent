#![allow(non_local_definitions)]
//! Renderer trait: форматирует финальный `AgentOutput` в текст для показа
//! пользователю.
//!
//! Renderer — первый trait в проекте с sabi-стабильным ABI. Для sabi_trait
//! требуется sync-версия trait'а (async не поддерживается), поэтому `render`
//! возвращает готовый `String`, не Future. Внутри реализации плагин может
//! использовать blocking I/O или локальный tokio runtime.
//!
//! ## ABI передача DTO
//!
//! DTO (`AgentOutput` и т.п.) передаются через границу в JSON-сериализованном
//! виде как `RString`. Плагин десериализует обратно через `serde_json`.
//! Это универсальный подход: не требует переделки DTO в `#[repr(C)]` и
//! работает для всех сериализуемых типов, включая `serde_json::Value`-поля.
//! Overhead сериализации минимален: Renderer вызывается раз за turn.
//!
//! Для удобства пользователя ядро/плагины работают с native DTO
//! (`AgentOutput`, `String`), сериализация/десериализация выполняется в
//! тонких wrapper'ах (`Renderer::render_native`).

use abi_stable::{
    StableAbi, sabi_trait,
    std_types::{RResult, RString},
};
use anyhow::Result;

use crate::domain::AgentOutput;

/// Ошибка рендеринга. Передаётся через границу плагина через `RString`.
#[repr(C)]
#[derive(StableAbi, Debug, Clone)]
#[non_exhaustive]
pub struct RenderError {
    pub message: RString,
}

impl RenderError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into().into(),
        }
    }
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl std::error::Error for RenderError {}

/// ABI-стабильный trait Renderer.
///
/// Метод `render` принимает `AgentOutput` в виде JSON-строки и возвращает
/// готовый текст. Это позволяет передавать любые DTO без переделки в
/// `#[repr(C)]`.
///
/// Реализации могут быть core stubs или плагинами через FFI.
#[sabi_trait]
pub trait Renderer: Send + Sync + 'static {
    /// Рендерит `AgentOutput` (сериализованный в JSON) в строку для показа.
    ///
    /// Реализации обычно вызывают `render_native` через хелпер.
    fn render_json(&self, output_json: RString) -> RResult<RString, RenderError>;
}

/// Ffi-safe trait object: `RBox<()>`-based, owned и `Send + Sync`.
///
/// Используется ядром и плагинами одинаково.
pub type RendererObject = Renderer_TO<abi_stable::std_types::RBox<()>>;

/// Удобный хелпер: позволяет рендерить `AgentOutput` напрямую через
/// `RendererObject`, скрывая JSON-сериализацию на границе.
///
/// Используется ядром; плагины могут использовать аналогичную обёртку
/// `render_native_in_plugin` внутри своей реализации `render_json`.
pub fn render_via_object(renderer: &RendererObject, output: &AgentOutput) -> Result<String> {
    let json: RString = serde_json::to_string(output)?.into();
    match renderer.render_json(json) {
        RResult::ROk(text) => Ok(text.into_string()),
        RResult::RErr(err) => Err(anyhow::anyhow!("renderer error: {}", err.message)),
    }
}

/// Симметричный хелпер для плагина: принимает RString с JSON, возвращает
/// `AgentOutput` для нативной реализации рендеринга.
pub fn parse_output_json(json: &str) -> Result<AgentOutput> {
    serde_json::from_str(json).map_err(Into::into)
}
