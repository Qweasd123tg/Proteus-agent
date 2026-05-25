//! `find_files` tool: поиск файлов по glob через ripgrep `--files`.

use std::{path::Path, process::Command, time::Duration};

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::{Value, json};

use crate::util::{
    err_result, ok_result, optional_positive_usize, optional_string_array, parse_call,
    plugin_error, required_string, run_lines_limited, workspace_path,
};

const FIND_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_MAX_RESULTS: usize = 100;
const MAX_MAX_RESULTS: usize = 1_000;

pub struct FindFilesTool;

impl PluginTool for FindFilesTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "find_files",
            "description": "Find workspace files by glob pattern using ripgrep --files. Prefer this over shell find/ls for read-only file discovery.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern, for example \"**/*.rs\" or \"crates/**/Cargo.toml\"."
                    },
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative search root. Defaults to workspace root."
                    },
                    "exclude": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional glob patterns to exclude."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum paths to return. Defaults to 100 and is capped at 1000."
                    }
                },
                "required": ["pattern"]
            },
            "safety": "ReadOnly",
            "timeout_ms": FIND_TIMEOUT.as_millis() as u64,
            "metadata": null
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
        let base = match std::fs::canonicalize(cwd_path) {
            Ok(path) => path,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to canonicalize cwd {}: {e}", cwd_path.display()),
                );
            }
        };
        let root_arg = call.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let root = match workspace_path(&base, Path::new(root_arg)) {
            Ok(path) => path,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let root_relative = match root.strip_prefix(&base) {
            Ok(path) if path.as_os_str().is_empty() => ".".to_owned(),
            Ok(path) => path.display().to_string(),
            Err(_) => return err_result(&call.id, &call.name, "path escapes workspace".to_owned()),
        };
        let max_results = match optional_positive_usize(&call.args, "max_results", &call.name) {
            Ok(value) => value.unwrap_or(DEFAULT_MAX_RESULTS).min(MAX_MAX_RESULTS),
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let exclude = match optional_string_array(&call.args, "exclude", &call.name) {
            Ok(value) => value,
            Err(e) => return err_result(&call.id, &call.name, e),
        };

        let mut command = Command::new("rg");
        command
            .current_dir(&base)
            .arg("--files")
            .arg("--glob")
            .arg(&pattern);
        for pattern in &exclude {
            command.arg("--glob").arg(format!("!{pattern}"));
        }
        command
            .arg("--")
            .arg(&root_relative)
            .stdin(std::process::Stdio::null());

        let lines = match run_lines_limited(command, max_results, FIND_TIMEOUT) {
            Ok(lines) => lines,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to run ripgrep files search: {e} (is 'rg' installed?)"),
                );
            }
        }
        .into_iter()
        .map(normalize_rg_file_path)
        .collect::<Vec<_>>();
        let match_count = lines.len();
        let output = if lines.is_empty() {
            "(no matches)".to_owned()
        } else {
            lines.join("\n")
        };
        ok_result(
            &call.id,
            &call.name,
            output,
            json!({
                "pattern": pattern,
                "path": root.display().to_string(),
                "exclude": exclude,
                "match_count": match_count,
                "max_results": max_results,
                "truncated": match_count >= max_results,
            }),
        )
    }
}

fn normalize_rg_file_path(path: String) -> String {
    path.strip_prefix("./").unwrap_or(&path).to_owned()
}
