//! Публичные trait'ы, DTO и canonical model standard для modular-agent.
//!
//! Этот crate — стабильный API для плагинов. Плагины depend на agent-contracts,
//! ядро (modular-agent) также depend на agent-contracts.

pub mod contracts;
pub mod domain;
pub mod model_standard;
