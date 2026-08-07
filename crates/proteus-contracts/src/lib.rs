//! Публичные trait'ы, DTO и canonical model standard для proteus-core.
//!
//! Этот crate содержит canonical contracts, общие для host и process workers.
//! Внешние workers не зависят от `proteus-core`.

pub mod app_protocol;
pub mod contracts;
pub mod domain;
pub mod model_standard;
pub mod process_module;
pub mod tool_support;
