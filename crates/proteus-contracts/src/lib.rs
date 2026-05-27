//! Публичные trait'ы, DTO и canonical model standard для proteus-core.
//!
//! Этот crate — стабильный API для плагинов. Плагины depend на proteus-contracts,
//! ядро (proteus-core) также depend на proteus-contracts.

pub mod app_protocol;
pub mod contracts;
pub mod domain;
pub mod model_standard;
pub mod plugin;
pub mod tool_support;

/// Re-export `abi_stable` для плагинов и ядра: все используют одну и ту же
/// версию crate, что гарантирует ABI-совместимость.
pub use abi_stable;
