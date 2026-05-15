use std::path::{Component, Path, PathBuf};

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::PluginToolError;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
pub(crate) struct ToolCallDto {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

pub(crate) fn parse_call(call_json: &str) -> Result<ToolCallDto, String> {
    serde_json::from_str(call_json).map_err(|e| format!("failed to parse ToolCall: {e}"))
}

pub(crate) fn ok_result(
    call_id: &str,
    tool_name: &str,
    output: String,
    metadata: Value,
) -> RResult<RString, PluginToolError> {
    let result = json!({
        "call_id": call_id,
        "ok": true,
        "output": output,
        "content": [],
        "error": null,
        "metadata": with_tool(metadata, tool_name),
    });
    RResult::ROk(RString::from(result.to_string()))
}

pub(crate) fn err_result(
    call_id: &str,
    tool_name: &str,
    error: String,
) -> RResult<RString, PluginToolError> {
    let result = json!({
        "call_id": call_id,
        "ok": false,
        "output": "",
        "content": [],
        "error": error,
        "metadata": { "tool": tool_name },
    });
    RResult::ROk(RString::from(result.to_string()))
}

pub(crate) fn plugin_error(message: String) -> RResult<RString, PluginToolError> {
    RResult::RErr(PluginToolError::new(message))
}

fn with_tool(mut metadata: Value, tool_name: &str) -> Value {
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("tool".to_owned(), json!(tool_name));
        Value::Object(obj.clone())
    } else {
        json!({ "tool": tool_name })
    }
}

pub(crate) fn required_string<'a>(
    args: &'a Value,
    key: &str,
    tool_name: &str,
) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("tool '{tool_name}' requires string arg '{key}'"))
}

pub(crate) fn optional_positive_usize(
    args: &Value,
    key: &str,
    tool_name: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let Some(number) = value.as_u64() else {
        return Err(format!("tool '{tool_name}' requires integer arg '{key}'"));
    };
    if number == 0 {
        return Err(format!(
            "tool '{tool_name}' requires positive integer arg '{key}'"
        ));
    }
    Ok(Some(number as usize))
}

pub(crate) fn workspace_path(cwd: &Path, path: &Path) -> Result<PathBuf, String> {
    let base = std::fs::canonicalize(cwd)
        .map_err(|e| format!("failed to canonicalize cwd {}: {e}", cwd.display()))?;
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let canonical = std::fs::canonicalize(&target)
        .map_err(|e| format!("failed to canonicalize {}: {e}", target.display()))?;
    if !canonical.starts_with(&base) {
        return Err(format!("path escapes workspace: {}", path.display()));
    }
    Ok(canonical)
}

pub(crate) fn workspace_path_for_write(cwd: &Path, path: &Path) -> Result<PathBuf, String> {
    let base = std::fs::canonicalize(cwd)
        .map_err(|e| format!("failed to canonicalize cwd {}: {e}", cwd.display()))?;
    let relative = if path.is_absolute() {
        path.strip_prefix(&base)
            .map_err(|_| format!("path escapes workspace: {}", path.display()))?
    } else {
        path
    };
    let safe_relative = safe_relative_path(relative)?;
    let target = base.join(&safe_relative);
    let parent = target
        .parent()
        .ok_or_else(|| format!("no parent for {}", target.display()))?;
    ensure_workspace_dirs(&base, parent)?;
    if let Ok(canonical_parent) = std::fs::canonicalize(parent) {
        if !canonical_parent.starts_with(&base) {
            return Err(format!("path escapes workspace: {}", path.display()));
        }
    }
    if let Ok(metadata) = std::fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "refusing to write through symlink: {}",
                path.display()
            ));
        }
        let canonical_target = std::fs::canonicalize(&target)
            .map_err(|e| format!("failed to canonicalize target {}: {e}", target.display()))?;
        if !canonical_target.starts_with(&base) {
            return Err(format!("path escapes workspace: {}", path.display()));
        }
    }
    if target.file_name().is_none() {
        return Err(format!("no file name in {}", target.display()));
    }
    Ok(target)
}

fn safe_relative_path(path: &Path) -> Result<PathBuf, String> {
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("path escapes workspace: {}", path.display()));
            }
        }
    }
    if safe.as_os_str().is_empty() {
        return Err(format!("no file name in {}", path.display()));
    }
    Ok(safe)
}

fn ensure_workspace_dirs(base: &Path, parent: &Path) -> Result<(), String> {
    let relative_parent = parent
        .strip_prefix(base)
        .map_err(|_| format!("path escapes workspace: {}", parent.display()))?;
    let mut current = base.to_path_buf();
    for component in relative_parent.components() {
        let Component::Normal(part) = component else {
            return Err(format!("path escapes workspace: {}", parent.display()));
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "refusing to create directory through symlink: {}",
                    current.display()
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!(
                    "path parent is not a directory: {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|e| {
                    format!("failed to create directory {}: {e}", current.display())
                })?;
            }
            Err(error) => {
                return Err(format!("failed to inspect {}: {error}", current.display()));
            }
        }
    }
    Ok(())
}
