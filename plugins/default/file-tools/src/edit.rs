//! `edit_file` tool: точечная замена текста в существующем файле
//! (opencode edit shape: old_string → new_string, опционально replace_all).

use std::path::Path;

use proteus_contracts::abi_stable::std_types::{RResult, RString};
use proteus_contracts::plugin::{PluginTool, PluginToolError, PluginToolHostMut};
use serde_json::json;

use crate::util::{
    err_result, ok_result, parse_call, parse_invocation_context, plugin_error, required_string,
    workspace_path,
};

const MAX_EDIT_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub struct EditFileTool;

impl PluginTool for EditFileTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "edit_file",
            "description": "Replace an exact text snippet in an existing file inside the workspace. `old_string` must match the file content exactly (including whitespace and indentation) and must be unique unless `replace_all` is true. Use `write_file` to create new files or fully rewrite existing ones.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string", "description": "Exact text to replace." },
                    "new_string": { "type": "string", "description": "Replacement text (must differ from old_string)." },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences of old_string (default false)." }
                },
                "required": ["path", "old_string", "new_string"]
            },
            "safety": "WritesFiles",
            "timeout_ms": 60000,
            "metadata": {
                "category": "filesystem",
                "tags": ["filesystem", "edit", "file", "write"],
                "aliases": ["edit file", "replace text", "modify file"],
                "approval": {
                    "cache_scopes": ["workspace_write"]
                }
            }
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(
        &self,
        call_json: RString,
        context_json: RString,
        _host: &mut PluginToolHostMut<'_>,
    ) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(c) => c,
            Err(e) => return plugin_error(e),
        };
        let context = match parse_invocation_context(context_json.as_str()) {
            Ok(context) => context,
            Err(error) => return plugin_error(error),
        };

        let path_str = match required_string(&call.args, "path", &call.name) {
            Ok(p) => p,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let old_string = match required_string(&call.args, "old_string", &call.name) {
            Ok(s) => s,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let new_string = match required_string(&call.args, "new_string", &call.name) {
            Ok(s) => s,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let replace_all = call
            .args
            .get("replace_all")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        if old_string.is_empty() {
            return err_result(
                &call.id,
                &call.name,
                "old_string must not be empty; use write_file to create a new file".to_owned(),
            );
        }
        if old_string == new_string {
            return err_result(
                &call.id,
                &call.name,
                "no changes to apply: old_string and new_string are identical".to_owned(),
            );
        }

        let cwd_path = context.cwd.as_path();
        let target_path = match workspace_path(cwd_path, Path::new(path_str)) {
            Ok(p) => p,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        match std::fs::metadata(&target_path) {
            Ok(metadata) if metadata.is_dir() => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("path is a directory, not a file: {}", target_path.display()),
                );
            }
            Ok(metadata) if metadata.len() > MAX_EDIT_FILE_BYTES => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!(
                        "file is too large to edit ({} bytes > {MAX_EDIT_FILE_BYTES})",
                        metadata.len()
                    ),
                );
            }
            Ok(_) => {}
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to stat {}: {e}", target_path.display()),
                );
            }
        }

        let content = match std::fs::read_to_string(&target_path) {
            Ok(content) => content,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to read {}: {e}", target_path.display()),
                );
            }
        };

        let match_count = content.matches(old_string).count();
        if match_count == 0 {
            return err_result(
                &call.id,
                &call.name,
                "old_string not found in file content; make sure it matches exactly, including whitespace".to_owned(),
            );
        }
        if match_count > 1 && !replace_all {
            return err_result(
                &call.id,
                &call.name,
                format!(
                    "old_string matches {match_count} locations; provide more surrounding context to make it unique or set replace_all"
                ),
            );
        }

        let (updated, replacements) = if replace_all {
            (content.replace(old_string, new_string), match_count)
        } else {
            (content.replacen(old_string, new_string, 1), 1)
        };

        if let Err(e) = std::fs::write(&target_path, &updated) {
            return err_result(
                &call.id,
                &call.name,
                format!("failed to write {}: {e}", target_path.display()),
            );
        }

        let metadata = json!({
            "path": target_path.display().to_string(),
            "replacements": replacements,
            "bytes_written": updated.len(),
        });
        ok_result(
            &call.id,
            &call.name,
            format!(
                "Replaced {replacements} occurrence(s) in {}",
                target_path.display()
            ),
            metadata,
        )
    }
}
