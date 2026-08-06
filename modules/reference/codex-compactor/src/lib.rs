//! Codex-style request-time history compactor.
//!
//! The upstream Codex compactor uses a model call to create the summary, then
//! replaces history with recent user messages plus a prefixed handoff summary.
//! This plugin follows that shape through Proteus' narrow compactor host: it
//! can request a model completion, but it cannot execute tools, mutate memory,
//! or rewrite the durable session log.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

mod budget;
mod compaction;
mod history;
mod summary;

use compaction::compact;
use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    contracts::CompactionInput,
    plugin::{PluginCompactionError, PluginCompactorHostMut, PluginHistoryCompactor},
};
#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::{
    abi_stable::{
        export_root_module, prefix_type::PrefixTypeTrait, sabi_trait::TD_Opaque, std_types::RStr,
    },
    plugin::{
        CompactorObject, PluginHistoryCompactor_TO, PluginRegisterError, PluginRegistryMut,
        PluginRoot, PluginRoot_Ref,
    },
};

pub(crate) const MODULE_ID: &str = "codex";

#[derive(Default)]
pub struct CodexCompactorPlugin;

impl PluginHistoryCompactor for CodexCompactorPlugin {
    fn compact_json(
        &self,
        input_json: RString,
        host: &mut PluginCompactorHostMut<'_>,
    ) -> RResult<RString, PluginCompactionError> {
        let input: CompactionInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return compaction_err(error),
        };

        match compact(input, host) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => compaction_err(error),
            },
            Err(error) => compaction_err(error),
        }
    }
}

fn compaction_err(error: impl std::fmt::Display) -> RResult<RString, PluginCompactionError> {
    RResult::RErr(PluginCompactionError::new(error.to_string()))
}

#[cfg(feature = "plugin-entrypoint")]
extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let compactor: CompactorObject =
        PluginHistoryCompactor_TO::from_value(CodexCompactorPlugin, TD_Opaque);
    registry.register_compactor(RString::from(MODULE_ID), compactor)
}

#[cfg(feature = "plugin-entrypoint")]
#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("codex-compactor"),
        description: RStr::from_str("Codex-style request-time history compactor"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests;
