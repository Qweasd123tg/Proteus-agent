//! `read_many_files` tool: пакетное чтение нескольких UTF-8 файлов.

use std::{io::Read, path::Path};

use proteus_contracts::abi_stable::std_types::{RResult, RString};
use proteus_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::{Value, json};

use crate::util::{
    err_result, ok_result, optional_positive_usize, parse_call, plugin_error, workspace_path,
};

const DEFAULT_MAX_BYTES_TOTAL: usize = 60 * 1024;
const MAX_MAX_BYTES_TOTAL: usize = 200 * 1024;
const DEFAULT_MAX_BYTES_PER_FILE: usize = 40 * 1024;
const MAX_MAX_BYTES_PER_FILE: usize = 100 * 1024;
const MAX_FILES: usize = 20;

pub struct ReadManyFilesTool;

impl PluginTool for ReadManyFilesTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "read_many_files",
            "description": "Read multiple UTF-8 files inside the current workspace with a shared byte budget.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Workspace-relative file paths to read. Maximum 20 files."
                    },
                    "max_bytes_total": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Total output byte budget. Defaults to 61440 and is capped at 204800."
                    },
                    "max_bytes_per_file": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Per-file byte budget. Defaults to 40960 and is capped at 102400."
                    },
                    "line_numbers": {
                        "type": "boolean",
                        "description": "Prefix each returned line with its 1-based line number."
                    }
                },
                "required": ["paths"]
            },
            "safety": "ReadOnly",
            "timeout_ms": 60000,
            "metadata": {
                "hot": true,
                "category": "filesystem",
                "tags": ["filesystem", "read", "files", "code"],
                "aliases": ["read files", "open multiple files", "inspect files"]
            }
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(c) => c,
            Err(e) => return plugin_error(e),
        };

        let paths = match required_paths(&call.args, &call.name) {
            Ok(paths) => paths,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        if paths.len() > MAX_FILES {
            return err_result(
                &call.id,
                &call.name,
                format!("tool '{}' accepts at most {MAX_FILES} paths", call.name),
            );
        }
        let max_bytes_total =
            match optional_positive_usize(&call.args, "max_bytes_total", &call.name) {
                Ok(value) => value
                    .unwrap_or(DEFAULT_MAX_BYTES_TOTAL)
                    .min(MAX_MAX_BYTES_TOTAL),
                Err(e) => return err_result(&call.id, &call.name, e),
            };
        let max_bytes_per_file =
            match optional_positive_usize(&call.args, "max_bytes_per_file", &call.name) {
                Ok(value) => value
                    .unwrap_or(DEFAULT_MAX_BYTES_PER_FILE)
                    .min(MAX_MAX_BYTES_PER_FILE),
                Err(e) => return err_result(&call.id, &call.name, e),
            };
        let line_numbers = call
            .args
            .get("line_numbers")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let cwd_path = Path::new(cwd.as_str());
        let mut sections = Vec::new();
        let mut files = Vec::new();
        let mut remaining = max_bytes_total;
        let mut total_original_bytes = 0usize;
        let mut total_returned_bytes = 0usize;
        let mut stopped_by_budget = false;
        let mut any_file_truncated = false;

        for requested_path in paths {
            if remaining == 0 {
                stopped_by_budget = true;
                break;
            }
            let absolute = match workspace_path(cwd_path, Path::new(&requested_path)) {
                Ok(path) => path,
                Err(e) => return err_result(&call.id, &call.name, e),
            };
            let metadata = match std::fs::metadata(&absolute) {
                Ok(metadata) => metadata,
                Err(e) => {
                    return err_result(
                        &call.id,
                        &call.name,
                        format!("failed to inspect {}: {e}", absolute.display()),
                    );
                }
            };
            if metadata.is_dir() {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("path is a directory; use list_dir to list entries: {requested_path}"),
                );
            }

            let limit = remaining.min(max_bytes_per_file);
            let original_bytes = metadata.len().try_into().unwrap_or(usize::MAX);
            let file_read = match read_text_prefix(&absolute, limit, original_bytes) {
                Ok(read) => read,
                Err(e) => return err_result(&call.id, &call.name, e),
            };
            let mut rendered = if line_numbers {
                add_line_numbers(&file_read.text)
            } else {
                file_read.text
            };
            let rendered_truncated_by_budget = rendered.len() > remaining;
            if rendered_truncated_by_budget {
                rendered = match utf8_prefix(rendered.as_bytes(), remaining) {
                    Ok(prefix) => prefix,
                    Err(e) => {
                        return err_result(
                            &call.id,
                            &call.name,
                            format!(
                                "failed to truncate rendered output for {} as UTF-8: {e}",
                                absolute.display()
                            ),
                        );
                    }
                };
            }
            let returned_bytes = rendered.len();
            let file_truncated = file_read.truncated || rendered_truncated_by_budget;
            any_file_truncated |= file_truncated;
            total_original_bytes += file_read.original_bytes;
            total_returned_bytes += returned_bytes;
            remaining = remaining.saturating_sub(returned_bytes);

            sections.push(format!("== {requested_path} ==\n{rendered}"));
            files.push(json!({
                "path": requested_path,
                "original_bytes": file_read.original_bytes,
                "returned_bytes": returned_bytes,
                "truncated": file_truncated,
            }));
            if file_truncated && remaining == 0 {
                stopped_by_budget = true;
                break;
            }
        }

        let output = if sections.is_empty() {
            "(no files read)".to_owned()
        } else {
            sections.join("\n\n")
        };
        ok_result(
            &call.id,
            &call.name,
            output,
            json!({
                "files": files,
                "file_count": files.len(),
                "max_files": MAX_FILES,
                "max_bytes_total": max_bytes_total,
                "max_bytes_per_file": max_bytes_per_file,
                "line_numbers": line_numbers,
                "total_original_bytes": total_original_bytes,
                "total_returned_bytes": total_returned_bytes,
                "truncated": stopped_by_budget || any_file_truncated,
            }),
        )
    }
}

#[derive(Debug)]
struct FileRead {
    text: String,
    original_bytes: usize,
    truncated: bool,
}

fn required_paths(args: &Value, tool_name: &str) -> Result<Vec<String>, String> {
    let Some(value) = args.get("paths") else {
        return Err(format!("tool '{tool_name}' requires array arg 'paths'"));
    };
    let Some(paths) = value.as_array() else {
        return Err(format!("tool '{tool_name}' requires array arg 'paths'"));
    };
    if paths.is_empty() {
        return Err(format!("tool '{tool_name}' requires at least one path"));
    }
    let mut result = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(path) = path.as_str() else {
            return Err(format!(
                "tool '{tool_name}' requires array arg 'paths' to contain only strings"
            ));
        };
        result.push(path.to_owned());
    }
    Ok(result)
}

fn read_text_prefix(path: &Path, limit: usize, original_bytes: usize) -> Result<FileRead, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let mut bytes = Vec::with_capacity(limit.saturating_add(1));
    file.by_ref()
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let truncated = bytes.len() > limit || original_bytes > bytes.len();
    if truncated {
        bytes.truncate(limit);
    }
    let text = utf8_prefix(&bytes, bytes.len())
        .map_err(|e| format!("failed to decode {} as UTF-8: {e}", path.display()))?;
    Ok(FileRead {
        text,
        original_bytes,
        truncated,
    })
}

fn utf8_prefix(bytes: &[u8], max_bytes: usize) -> Result<String, std::str::Utf8Error> {
    let mut end = bytes.len().min(max_bytes);
    loop {
        match std::str::from_utf8(&bytes[..end]) {
            Ok(text) => return Ok(text.to_owned()),
            Err(error) if error.error_len().is_none() && end > 0 => {
                end = error.valid_up_to();
            }
            Err(error) => return Err(error),
        }
    }
}

fn add_line_numbers(text: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(index, line)| format!("{}\t{}", index + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}
