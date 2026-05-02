//! File tools plugin: read_file, write_file, list_dir, grep.
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
//! После этого добавьте нужные имена (`read_file`, `write_file`, `list_dir`,
//! `grep`) в `tools.enabled`. Установленный плагин расширяет namespace, но
//! tools остаются opt-in через config.

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

#[cfg(test)]
mod tests {
    use agent_contracts::{
        abi_stable::std_types::{RResult, RString},
        plugin::PluginTool,
    };
    use serde_json::{Value, json};

    use super::*;

    fn invoke<T: PluginTool>(tool: &T, cwd: &std::path::Path, args: Value) -> Value {
        let call = json!({
            "id": "call_test",
            "name": serde_json::from_str::<Value>(tool.spec_json().as_str())
                .expect("spec json")["name"]
                .as_str()
                .expect("tool name"),
            "args": args
        });
        match tool.invoke_json(
            RString::from(call.to_string()),
            RString::from(cwd.display().to_string()),
        ) {
            RResult::ROk(result) => serde_json::from_str(result.as_str()).expect("tool result"),
            RResult::RErr(err) => panic!("plugin error: {}", err.message),
        }
    }

    #[test]
    fn read_file_supports_line_ranges_and_line_numbers() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("sample.txt"), "one\ntwo\nthree\n").expect("sample");

        let result = invoke(
            &ReadFileTool,
            dir.path(),
            json!({
                "path": "sample.txt",
                "start_line": 2,
                "limit": 1,
                "line_numbers": true
            }),
        );

        assert_eq!(result["ok"], true);
        assert_eq!(result["output"], "2\ttwo");
        assert_eq!(result["metadata"]["start_line"], 2);
        assert_eq!(result["metadata"]["end_line"], 2);
        assert_eq!(result["metadata"]["truncated"], true);
    }

    #[test]
    fn write_file_creates_file_inside_workspace() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = invoke(
            &WriteFileTool,
            dir.path(),
            json!({
                "path": "notes/out.txt",
                "content": "hello"
            }),
        );

        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("failed to canonicalize parent"));

        std::fs::create_dir(dir.path().join("notes")).expect("notes dir");
        let result = invoke(
            &WriteFileTool,
            dir.path(),
            json!({
                "path": "notes/out.txt",
                "content": "hello"
            }),
        );

        assert_eq!(result["ok"], true);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("notes/out.txt")).expect("written file"),
            "hello"
        );
        assert_eq!(result["metadata"]["bytes_written"], 5);
    }

    #[test]
    fn list_dir_returns_sorted_entries_with_kind() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("b.txt"), "b").expect("b");
        std::fs::create_dir(dir.path().join("a_dir")).expect("a_dir");

        let result = invoke(&ListDirTool, dir.path(), json!({ "path": "." }));

        assert_eq!(result["ok"], true);
        assert_eq!(result["output"], "dir\ta_dir\nfile\tb.txt");
        assert_eq!(result["metadata"]["entry_count"], 2);
    }

    #[test]
    fn read_file_rejects_parent_escape() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = invoke(&ReadFileTool, dir.path(), json!({ "path": "../secret.txt" }));

        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("canonicalize"));
    }
}
