//! `read_file` tool: чтение файла целиком или по диапазону строк.

use std::path::{Path, PathBuf};

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::json;

use crate::util::{
    err_result, ok_result, optional_positive_usize, parse_call, plugin_error, required_string,
    workspace_path,
};

pub struct ReadFileTool;

impl PluginTool for ReadFileTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "read_file",
            "description": "Read a UTF-8 file inside the current workspace, optionally by line range",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "start_line": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "1-based first line to read. Defaults to 1."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of lines to return."
                    },
                    "line_numbers": {
                        "type": "boolean",
                        "description": "Prefix each returned line with its 1-based line number."
                    }
                },
                "required": ["path"]
            },
            "safety": "ReadOnly",
            "timeout_ms": 5000,
            "metadata": null
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(c) => c,
            Err(e) => return plugin_error(e),
        };

        let path_str = match required_string(&call.args, "path", &call.name) {
            Ok(p) => p,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let cwd_path = Path::new(cwd.as_str());
        let path = match workspace_path(cwd_path, Path::new(path_str)) {
            Ok(p) => p,
            Err(e) => return err_result(&call.id, &call.name, e),
        };

        let metadata_res = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to inspect {}: {e}", path.display()),
                );
            }
        };
        if metadata_res.is_dir() {
            return err_result(
                &call.id,
                &call.name,
                format!(
                    "path is a directory; use list_dir to list entries: {}",
                    path.display()
                ),
            );
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to read {}: {e}", path.display()),
                );
            }
        };

        let options = match ReadOptions::from_args(&call.args, &call.name) {
            Ok(o) => o,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let (output, metadata) = render_read_output(&content, &path, options);
        ok_result(&call.id, &call.name, output, metadata)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ReadOptions {
    start_line: Option<usize>,
    limit: Option<usize>,
    line_numbers: bool,
}

impl ReadOptions {
    fn from_args(args: &serde_json::Value, tool_name: &str) -> Result<Self, String> {
        Ok(Self {
            start_line: optional_positive_usize(args, "start_line", tool_name)?,
            limit: optional_positive_usize(args, "limit", tool_name)?,
            line_numbers: args
                .get("line_numbers")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }

    fn is_default(self) -> bool {
        self.start_line.is_none() && self.limit.is_none() && !self.line_numbers
    }
}

fn render_read_output(
    content: &str,
    path: &PathBuf,
    options: ReadOptions,
) -> (String, serde_json::Value) {
    let total_lines = content.lines().count();
    if options.is_default() {
        return (
            content.to_owned(),
            json!({
                "path": path.display().to_string(),
                "total_lines": total_lines,
                "returned_lines": total_lines,
                "truncated": false,
            }),
        );
    }

    let start_line = options.start_line.unwrap_or(1);
    let limit = options.limit.unwrap_or(usize::MAX);
    let start_index = start_line.saturating_sub(1);
    let mut returned = 0usize;
    let mut rendered = Vec::new();
    for (index, line) in content.lines().enumerate().skip(start_index).take(limit) {
        returned += 1;
        if options.line_numbers {
            rendered.push(format!("{}\t{}", index + 1, line));
        } else {
            rendered.push(line.to_owned());
        }
    }
    let end_line = if returned == 0 {
        None
    } else {
        Some(start_line + returned - 1)
    };
    let truncated = start_index + returned < total_lines;

    (
        rendered.join("\n"),
        json!({
            "path": path.display().to_string(),
            "start_line": start_line,
            "end_line": end_line,
            "limit": options.limit,
            "line_numbers": options.line_numbers,
            "total_lines": total_lines,
            "returned_lines": returned,
            "truncated": truncated,
        }),
    )
}
