//! `list_dir` tool: показывает содержимое директории внутри workspace.

use std::path::Path;

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::{Value, json};

use crate::util::{err_result, ok_result, parse_call, plugin_error, workspace_path};

pub struct ListDirTool;

impl PluginTool for ListDirTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "list_dir",
            "description": "List entries (files and subdirectories) inside a workspace directory",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path relative to workspace. Defaults to workspace root."
                    }
                },
                "required": []
            },
            "safety": "ReadOnly",
            "timeout_ms": 5000,
            "metadata": null
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(
        &self,
        call_json: RString,
        cwd: RString,
    ) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(c) => c,
            Err(e) => return plugin_error(e),
        };

        let path_str = call.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let cwd_path = Path::new(cwd.as_str());
        let target_path = match workspace_path(cwd_path, Path::new(path_str)) {
            Ok(p) => p,
            Err(e) => return err_result(&call.id, &call.name, e),
        };

        let metadata = match std::fs::metadata(&target_path) {
            Ok(m) => m,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to inspect {}: {e}", target_path.display()),
                );
            }
        };
        if !metadata.is_dir() {
            return err_result(
                &call.id,
                &call.name,
                format!("path is not a directory: {}", target_path.display()),
            );
        }

        let entries = match std::fs::read_dir(&target_path) {
            Ok(e) => e,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to read directory {}: {e}", target_path.display()),
                );
            }
        };

        let mut lines = Vec::new();
        let mut count = 0usize;
        for entry in entries.flatten() {
            count += 1;
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = match entry.file_type() {
                Ok(ft) if ft.is_dir() => "dir",
                Ok(ft) if ft.is_symlink() => "symlink",
                Ok(_) => "file",
                Err(_) => "unknown",
            };
            lines.push(format!("{kind}\t{name}"));
        }
        lines.sort();

        let output = lines.join("\n");
        let result_metadata = json!({
            "path": target_path.display().to_string(),
            "entry_count": count,
        });
        ok_result(&call.id, &call.name, output, result_metadata)
    }
}
