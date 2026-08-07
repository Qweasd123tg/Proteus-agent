//! File tools reference process module: read_file, write_file, list_dir, grep, find_files,
//! read_many_files.
//!
//! Reference-реализация файловых tools, экспортируемая единым process worker.
//! Она использует sync `ToolModule` + `std::fs` (не `tokio::fs`) и проверяет,
//! что поведение tools можно вынести за границу core.
//!
//! Этот crate не является шаблоном для новых modules: целевая граница —
//! process protocol из `docs/process-module-architecture.md`.
//!
//! ## Установка
//!
//! ```bash
//! cargo build --release -p file-tools
//! Реализация линкуется только внутрь `proteus-reference-worker`; host видит
//! её через process Tool contract v1.
//! ```
//!
//! После этого добавьте нужные имена (`read_file`, `write_file`, `list_dir`,
//! `grep`, `find_files`, `read_many_files`) в `tools.enabled`. Установленный
//! module расширяет namespace, но tools остаются opt-in через config.

mod edit;
mod find;
mod list;
mod read;
mod read_many;
mod search;
mod util;
mod write;

use proteus_contracts::process_module::{ModuleRegistry, ProcessModuleError, ToolModuleObject};

use crate::{
    edit::EditFileTool, find::FindFilesTool, list::ListDirTool, read::ReadFileTool,
    read_many::ReadManyFilesTool, search::GrepTool, write::WriteFileTool,
};

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let read: ToolModuleObject = Box::new(ReadFileTool);
    if let Err(err) = registry.register_tool(read) {
        return Err(err);
    }

    let write: ToolModuleObject = Box::new(WriteFileTool);
    if let Err(err) = registry.register_tool(write) {
        return Err(err);
    }

    let edit: ToolModuleObject = Box::new(EditFileTool);
    if let Err(err) = registry.register_tool(edit) {
        return Err(err);
    }

    let list: ToolModuleObject = Box::new(ListDirTool);
    if let Err(err) = registry.register_tool(list) {
        return Err(err);
    }

    let grep: ToolModuleObject = Box::new(GrepTool);
    if let Err(err) = registry.register_tool(grep) {
        return Err(err);
    }

    let find_files: ToolModuleObject = Box::new(FindFilesTool);
    if let Err(err) = registry.register_tool(find_files) {
        return Err(err);
    }

    let read_many: ToolModuleObject = Box::new(ReadManyFilesTool);
    if let Err(err) = registry.register_tool(read_many) {
        return Err(err);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use proteus_contracts::{
        contracts::ToolInvocationOwner,
        domain::{ToolSpec, new_session_id, new_thread_id, new_turn_id},
        process_module::{
            ProcessModuleError, ToolModule, ToolModuleHost, ToolModuleInvocationContext,
        },
    };
    use serde_json::{Value, json};

    use super::*;
    use crate::{find::FindFilesTool, read_many::ReadManyFilesTool};

    struct TestToolHost;

    impl ToolModuleHost for TestToolHost {
        fn is_cancelled(&self) -> Result<bool, ProcessModuleError> {
            Ok(false)
        }
    }

    fn invoke<T: ToolModule>(tool: &T, cwd: &std::path::Path, args: Value) -> Value {
        let call = json!({
            "id": "call_test",
            "name": serde_json::from_str::<Value>(tool.spec_json().as_str())
                .expect("spec json")["name"]
                .as_str()
                .expect("tool name"),
            "args": args
        });
        let context = ToolModuleInvocationContext {
            cwd: cwd.to_path_buf(),
            owner: ToolInvocationOwner::new(new_session_id(), new_thread_id(), new_turn_id()),
            config: json!({}),
        };
        let mut host = TestToolHost;
        match tool.invoke_json(
            call.to_string(),
            serde_json::to_string(&context).expect("context json"),
            &mut host,
        ) {
            Ok(result) => serde_json::from_str(result.as_str()).expect("tool result"),
            Err(err) => panic!("module error: {}", err.message),
        }
    }

    fn spec<T: ToolModule>(tool: &T) -> Value {
        serde_json::from_str(tool.spec_json().as_str()).expect("spec json")
    }

    fn assert_canonical_spec<T: ToolModule>(tool: &T) {
        serde_json::from_str::<ToolSpec>(tool.spec_json().as_str())
            .expect("module spec must match strict ToolSpec");
    }

    #[test]
    fn every_file_tool_emits_strict_canonical_spec() {
        assert_canonical_spec(&ReadFileTool);
        assert_canonical_spec(&WriteFileTool);
        assert_canonical_spec(&EditFileTool);
        assert_canonical_spec(&ListDirTool);
        assert_canonical_spec(&GrepTool);
        assert_canonical_spec(&FindFilesTool);
        assert_canonical_spec(&ReadManyFilesTool);
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
    fn edit_file_replaces_unique_match() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("main.rs"), "fn main() {\n    old();\n}\n").expect("file");

        let result = invoke(
            &EditFileTool,
            dir.path(),
            json!({
                "path": "main.rs",
                "old_string": "    old();",
                "new_string": "    new();"
            }),
        );

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["replacements"], 1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.rs")).expect("edited"),
            "fn main() {\n    new();\n}\n"
        );
    }

    #[test]
    fn edit_file_requires_unique_match_unless_replace_all() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("dup.txt"), "x\nx\n").expect("file");

        let ambiguous = invoke(
            &EditFileTool,
            dir.path(),
            json!({ "path": "dup.txt", "old_string": "x", "new_string": "y" }),
        );
        assert_eq!(ambiguous["ok"], false);
        assert!(
            ambiguous["error"]
                .as_str()
                .unwrap()
                .contains("matches 2 locations")
        );

        let replaced = invoke(
            &EditFileTool,
            dir.path(),
            json!({ "path": "dup.txt", "old_string": "x", "new_string": "y", "replace_all": true }),
        );
        assert_eq!(replaced["ok"], true);
        assert_eq!(replaced["metadata"]["replacements"], 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dup.txt")).expect("edited"),
            "y\ny\n"
        );
    }

    #[test]
    fn edit_file_errors_when_old_string_is_missing_or_degenerate() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("a.txt"), "content\n").expect("file");

        let not_found = invoke(
            &EditFileTool,
            dir.path(),
            json!({ "path": "a.txt", "old_string": "absent", "new_string": "x" }),
        );
        assert_eq!(not_found["ok"], false);
        assert!(not_found["error"].as_str().unwrap().contains("not found"));

        let identical = invoke(
            &EditFileTool,
            dir.path(),
            json!({ "path": "a.txt", "old_string": "content", "new_string": "content" }),
        );
        assert_eq!(identical["ok"], false);
        assert!(identical["error"].as_str().unwrap().contains("identical"));

        let empty = invoke(
            &EditFileTool,
            dir.path(),
            json!({ "path": "a.txt", "old_string": "", "new_string": "x" }),
        );
        assert_eq!(empty["ok"], false);
        assert!(empty["error"].as_str().unwrap().contains("write_file"));
    }

    #[test]
    fn edit_file_rejects_parent_escape_and_missing_file() {
        let dir = tempfile::tempdir().expect("workspace");

        let escape = invoke(
            &EditFileTool,
            dir.path(),
            json!({ "path": "../secret.txt", "old_string": "a", "new_string": "b" }),
        );
        assert_eq!(escape["ok"], false);
        assert!(escape["error"].as_str().unwrap().contains("canonicalize"));

        let missing = invoke(
            &EditFileTool,
            dir.path(),
            json!({ "path": "missing.txt", "old_string": "a", "new_string": "b" }),
        );
        assert_eq!(missing["ok"], false);
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
