//! File tools plugin: read_file, write_file, list_dir, grep, find_files,
//! read_many_files.
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
//! mkdir -p ~/.proteus/plugins/file-tools
//! cp target/release/libfile_tools.so ~/.proteus/plugins/file-tools/
//! cp plugins/default/file-tools/plugin.toml ~/.proteus/plugins/file-tools/
//! ```
//!
//! После этого добавьте нужные имена (`read_file`, `write_file`, `list_dir`,
//! `grep`, `find_files`, `read_many_files`) в `tools.enabled`. Установленный
//! плагин расширяет namespace, но tools остаются opt-in через config.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

mod find;
mod list;
mod read;
mod read_many;
mod search;
mod util;
mod write;

use proteus_contracts::{
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

use crate::{
    find::FindFilesTool, list::ListDirTool, read::ReadFileTool, read_many::ReadManyFilesTool,
    search::GrepTool, write::WriteFileTool,
};

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

    let find_files: PluginToolObject = PluginTool_TO::from_value(FindFilesTool, TD_Opaque);
    if let RResult::RErr(err) = registry.register_tool(find_files) {
        return RResult::RErr(err);
    }

    let read_many: PluginToolObject = PluginTool_TO::from_value(ReadManyFilesTool, TD_Opaque);
    if let RResult::RErr(err) = registry.register_tool(read_many) {
        return RResult::RErr(err);
    }

    RResult::ROk(())
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("file-tools"),
        description: RStr::from_str(
            "Basic file tools: read_file, write_file, list_dir, grep, find_files, read_many_files",
        ),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use proteus_contracts::{
        abi_stable::std_types::{RResult, RString},
        plugin::PluginTool,
    };
    use serde_json::{Value, json};

    use super::*;
    use crate::{find::FindFilesTool, read_many::ReadManyFilesTool};

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

    fn spec<T: PluginTool>(tool: &T) -> Value {
        serde_json::from_str(tool.spec_json().as_str()).expect("spec json")
    }

    #[test]
    fn file_tool_specs_allow_slow_filesystems_and_searches() {
        assert_eq!(spec(&ReadFileTool)["timeout_ms"], 60_000);
        assert_eq!(spec(&WriteFileTool)["timeout_ms"], 60_000);
        assert_eq!(spec(&ListDirTool)["timeout_ms"], 60_000);
        assert_eq!(spec(&GrepTool)["timeout_ms"], 60_000);
        assert_eq!(spec(&FindFilesTool)["timeout_ms"], 60_000);
        assert_eq!(spec(&ReadManyFilesTool)["timeout_ms"], 60_000);
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
    fn read_file_rejects_full_read_above_size_limit_but_allows_ranges() {
        let dir = tempfile::tempdir().expect("workspace");
        let content = "x\n".repeat(crate::read::MAX_READ_FILE_BYTES as usize / 2 + 1);
        std::fs::write(dir.path().join("large.txt"), content).expect("large file");

        let full = invoke(&ReadFileTool, dir.path(), json!({ "path": "large.txt" }));

        assert_eq!(full["ok"], false);
        assert!(full["error"].as_str().unwrap().contains("too large"));

        let ranged = invoke(
            &ReadFileTool,
            dir.path(),
            json!({
                "path": "large.txt",
                "start_line": 2,
                "limit": 2,
                "line_numbers": true
            }),
        );

        assert_eq!(ranged["ok"], true);
        assert_eq!(ranged["output"], "2\tx\n3\tx");
        assert_eq!(ranged["metadata"]["truncated"], true);
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
    fn read_many_files_reads_multiple_files_with_line_numbers() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").expect("a");
        std::fs::write(dir.path().join("b.txt"), "three\n").expect("b");

        let result = invoke(
            &ReadManyFilesTool,
            dir.path(),
            json!({
                "paths": ["a.txt", "b.txt"],
                "line_numbers": true,
                "max_bytes_total": 200
            }),
        );

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["file_count"], 2);
        let output = result["output"].as_str().unwrap();
        assert!(output.contains("== a.txt ==\n1\tone\n2\ttwo"), "{output}");
        assert!(output.contains("== b.txt ==\n1\tthree"), "{output}");
    }

    #[test]
    fn read_many_files_enforces_shared_budget() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("a.txt"), "abcd").expect("a");

        let result = invoke(
            &ReadManyFilesTool,
            dir.path(),
            json!({
                "paths": ["a.txt"],
                "max_bytes_total": 3
            }),
        );

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["truncated"], true);
        assert_eq!(result["metadata"]["files"][0]["returned_bytes"], 3);
    }

    #[test]
    fn read_many_files_reports_truncated_with_line_numbers_when_rendered_output_is_longer() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("a.txt"), "abcd").expect("a");

        let result = invoke(
            &ReadManyFilesTool,
            dir.path(),
            json!({
                "paths": ["a.txt"],
                "line_numbers": true,
                "max_bytes_total": 20,
                "max_bytes_per_file": 3
            }),
        );

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["files"][0]["truncated"], true);
        assert_eq!(result["metadata"]["truncated"], true);
        assert!(
            result["metadata"]["total_returned_bytes"].as_u64().unwrap()
                >= result["metadata"]["total_original_bytes"].as_u64().unwrap()
        );
    }

    #[test]
    fn find_files_returns_glob_matches() {
        if std::process::Command::new("rg")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::create_dir(dir.path().join("src")).expect("src");
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn ok() {}\n").expect("lib");
        std::fs::write(dir.path().join("src/skip.txt"), "skip\n").expect("skip");

        let result = invoke(
            &FindFilesTool,
            dir.path(),
            json!({
                "pattern": "**/*.rs",
                "max_results": 10
            }),
        );

        assert_eq!(result["ok"], true);
        assert_eq!(result["output"], "src/lib.rs");
        assert_eq!(result["metadata"]["match_count"], 1);
    }

    #[test]
    fn read_file_rejects_parent_escape() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = invoke(
            &ReadFileTool,
            dir.path(),
            json!({ "path": "../secret.txt" }),
        );

        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("canonicalize"));
    }

    #[test]
    fn read_many_files_rejects_parent_escape() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = invoke(
            &ReadManyFilesTool,
            dir.path(),
            json!({ "paths": ["../secret.txt"] }),
        );

        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("canonicalize"));
    }
}
