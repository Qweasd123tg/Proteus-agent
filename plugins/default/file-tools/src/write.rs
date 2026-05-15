//! `write_file` tool: создаёт или перезаписывает файл внутри workspace.

use std::path::Path;

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::json;

use crate::util::{
    err_result, ok_result, parse_call, plugin_error, required_string, workspace_path_for_write,
};

pub struct WriteFileTool;

impl PluginTool for WriteFileTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "write_file",
            "description": "Write UTF-8 content to a file inside the current workspace. Creates missing parent directories and overwrites existing files.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            },
            "safety": "WritesFiles",
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
        let content = match required_string(&call.args, "content", &call.name) {
            Ok(c) => c,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let cwd_path = Path::new(cwd.as_str());
        let target_path = match workspace_path_for_write(cwd_path, Path::new(path_str)) {
            Ok(p) => p,
            Err(e) => return err_result(&call.id, &call.name, e),
        };

        let bytes_written = content.len();
        if let Err(e) = std::fs::write(&target_path, content) {
            return err_result(
                &call.id,
                &call.name,
                format!("failed to write {}: {e}", target_path.display()),
            );
        }

        let metadata = json!({
            "path": target_path.display().to_string(),
            "bytes_written": bytes_written,
        });
        ok_result(
            &call.id,
            &call.name,
            format!("Wrote {bytes_written} bytes to {}", target_path.display()),
            metadata,
        )
    }
}
