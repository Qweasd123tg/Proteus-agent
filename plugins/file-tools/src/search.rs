//! `grep` tool: поиск по содержимому файлов через ripgrep.
//!
//! Требует установленный `rg` в `$PATH`. Если нет — tool всё ещё виден,
//! но возвращает ошибку при вызове. Это feature не bug — пусть модель
//! видит осмысленное сообщение "rg is not installed" вместо того чтобы
//! плагин молчал.

use std::path::Path;
use std::process::Command;

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::{Value, json};

use crate::util::{
    err_result, ok_result, optional_positive_usize, parse_call, plugin_error, required_string,
    workspace_path,
};

pub struct GrepTool;

impl PluginTool for GrepTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "grep",
            "description": "Search for a regex pattern in workspace files using ripgrep. Returns lines that match.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for (ripgrep syntax)."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in, relative to workspace. Defaults to workspace root."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum matching lines to return. Defaults to 50."
                    }
                },
                "required": ["pattern"]
            },
            "safety": "ReadOnly",
            "timeout_ms": 15000,
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

        let pattern = match required_string(&call.args, "pattern", &call.name) {
            Ok(p) => p.to_owned(),
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let cwd_path = Path::new(cwd.as_str());
        let search_path_arg = call.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let search_path = match workspace_path(cwd_path, Path::new(search_path_arg)) {
            Ok(p) => p,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let max_results = match optional_positive_usize(&call.args, "max_results", &call.name) {
            Ok(m) => m.unwrap_or(50),
            Err(e) => return err_result(&call.id, &call.name, e),
        };

        // rg с флагами: --no-heading --line-number, без цвета, один matcher.
        let rg_output = Command::new("rg")
            .arg("--no-heading")
            .arg("--line-number")
            .arg("--color=never")
            .arg("--max-count")
            .arg(max_results.to_string())
            .arg("--")
            .arg(&pattern)
            .arg(&search_path)
            .output();

        let output = match rg_output {
            Ok(o) => o,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to run ripgrep: {e} (is 'rg' installed?)"),
                );
            }
        };

        // rg exit codes: 0 = matches found, 1 = no matches, 2 = error.
        let status = output.status.code().unwrap_or(-1);
        if status == 2 {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return err_result(&call.id, &call.name, format!("ripgrep error: {stderr}"));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let lines: Vec<&str> = stdout.lines().take(max_results).collect();
        let match_count = lines.len();
        let truncated = stdout.lines().count() > match_count;

        let output_text = if lines.is_empty() {
            "(no matches)".to_owned()
        } else {
            lines.join("\n")
        };
        let metadata = json!({
            "pattern": pattern,
            "path": search_path.display().to_string(),
            "match_count": match_count,
            "max_results": max_results,
            "truncated": truncated,
        });
        ok_result(&call.id, &call.name, output_text, metadata)
    }
}
