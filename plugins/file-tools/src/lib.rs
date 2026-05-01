//! File tools plugin: read_file, write_file, list_dir.
//!
//! Плагин-версия базовых файловых tools. Логика та же что у builtin-версий
//! в ядре, но через sync `PluginTool` + `std::fs` (не `tokio::fs`).
//!
//! Цель плагина — показать что builtin tools можно вынести из ядра в плагин
//! без потери функциональности. В Волне 3 ядро будет содержать только
//! fallback-stubs, всё остальное — плагины. Этот плагин — шаблон для такой
//! миграции.
//!
//! ## Установка
//!
//! ```bash
//! cargo build --release -p file-tools
//! mkdir -p ~/.agent/plugins/file-tools
//! cp target/release/libfile_tools.so ~/.agent/plugins/file-tools/
//! cp plugins/file-tools/plugin.toml ~/.agent/plugins/file-tools/
//! ```
//!
//! После этого в config можно **не указывать** `read_file`/`write_file`/
//! `list_dir` в `tools.enabled` — плагин сам зарегистрирует их, и модель
//! их увидит.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

mod list;
mod read;
mod search;
mod util;
mod write;

use agent_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr},
    },
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginTool_TO,
        PluginToolObject,
    },
};

use crate::{list::ListDirTool, read::ReadFileTool, search::GrepTool, write::WriteFileTool};

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let read: PluginToolObject = PluginTool_TO::from_value(ReadFileTool, TD_Opaque);
    if let RResult::RErr(err) = registry.register_tool(read) {
        return RResult::RErr(err);
    }

    let write: PluginToolObject = PluginTool_TO::from_value(WriteFileTool, TD_Opaque);
    if let RResult::RErr(err) = registry.register_tool(write) {
        return RResult::RErr(err);
    }

    let list: PluginToolObject = PluginTool_TO::from_value(ListDirTool, TD_Opaque);
    if let RResult::RErr(err) = registry.register_tool(list) {
        return RResult::RErr(err);
    }

    let grep: PluginToolObject = PluginTool_TO::from_value(GrepTool, TD_Opaque);
    if let RResult::RErr(err) = registry.register_tool(grep) {
        return RResult::RErr(err);
    }

    RResult::ROk(())
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("file-tools"),
        description: RStr::from_str("Basic file tools: read_file, write_file, list_dir, grep"),
        register_modules,
    }
    .leak_into_prefix()
}
