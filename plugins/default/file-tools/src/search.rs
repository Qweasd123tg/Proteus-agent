//! `grep` tool: поиск по содержимому файлов через ripgrep.
//!
//! Требует установленный `rg` в `$PATH`. Если нет — tool всё ещё виден,
//! но возвращает ошибку при вызове. Это feature не bug — пусть модель
//! видит осмысленное сообщение "rg is not installed" вместо того чтобы
//! плагин молчал.

use std::{path::Path, process::Command, time::Duration};

use proteus_contracts::abi_stable::std_types::{RResult, RString};
use proteus_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::{Value, json};

use crate::util::{
    err_result, ok_result, optional_positive_usize, parse_call, plugin_error, required_string,
    run_lines_limited, workspace_path,
};

pub struct GrepTool;
const RG_TIMEOUT: Duration = Duration::from_secs(60);

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
            "timeout_ms": RG_TIMEOUT.as_millis() as u64,
            "metadata": {
                "hot": true,
                "category": "filesystem",
                "tags": ["filesystem", "search", "grep", "regex", "code"],
                "aliases": ["ripgrep", "find text", "search code", "search files"]
            }
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
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

        let mut command = Command::new("rg");
        command
            .arg("--no-heading")
            .arg("--line-number")
            .arg("--color=never")
            .arg("--max-columns")
            .arg("2000")
            .arg("--max-filesize")
            .arg("1M")
            .arg("--")
            .arg(&pattern)
            .arg(&search_path)
            .stdin(std::process::Stdio::null());

        let lines = match run_lines_limited(command, max_results, RG_TIMEOUT) {
            Ok(lines) => lines,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to run ripgrep: {e} (is 'rg' installed?)"),
                );
            }
        };

        let match_count = lines.len();
        let truncated = match_count >= max_results;

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
