//! Публичные trait'ы, DTO и canonical model standard для modular-agent.
//!
//! Этот crate — стабильный API для плагинов. Плагины depend на agent-contracts,
//! ядро (modular-agent) также depend на agent-contracts.

pub mod app_protocol;
pub mod contracts;
pub mod domain;
pub mod model_standard;
pub mod plugin;
pub mod tool_support;

/// Re-export `abi_stable` для плагинов и ядра: все используют одну и ту же
/// версию crate, что гарантирует ABI-совместимость.
pub use abi_stable;
