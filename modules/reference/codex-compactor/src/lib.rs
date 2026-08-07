//! Codex-style request-time history compactor.
//!
//! The upstream Codex compactor uses a model call to create the summary, then
//! replaces history with recent user messages plus a prefixed handoff summary.
//! This module follows that shape through Proteus' narrow compactor host: it
//! can request a model completion, but it cannot execute tools, mutate memory,
//! or rewrite the durable session log.

mod budget;
mod compaction;
mod history;
mod summary;

use compaction::compact;
use proteus_contracts::{
    contracts::CompactionInput,
    process_module::{
        CompactorModule, CompactorModuleHostMut, CompactorModuleObject, ModuleRegistry,
        ProcessModuleError,
    },
};

pub(crate) const MODULE_ID: &str = "codex";

#[derive(Default)]
pub struct CodexCompactorModule;

impl CompactorModule for CodexCompactorModule {
    fn compact_json(
        &self,
        input_json: String,
        host: &mut CompactorModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let input: CompactionInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return compaction_err(error),
        };

        match compact(input, host) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => Ok(String::from(json)),
                Err(error) => compaction_err(error),
            },
            Err(error) => compaction_err(error),
        }
    }
}

fn compaction_err(error: impl std::fmt::Display) -> Result<String, ProcessModuleError> {
    Err(ProcessModuleError::new(error.to_string()))
}

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let compactor: CompactorModuleObject = Box::new(CodexCompactorModule);
    registry.register_compactor(String::from(MODULE_ID), compactor)
}

#[cfg(test)]
mod tests;
