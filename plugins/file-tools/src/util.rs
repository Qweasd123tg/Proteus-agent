//! Общие утилиты для всех file-tools: workspace containment, парсинг аргументов,
//! сериализация результатов.

use std::path::{Path, PathBuf};

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::PluginToolError;
use serde::Deserialize;
use serde_json::{Value, json};

/// Входные данные tool: разбор `ToolCall` JSON.
#[derive(Debug, Deserialize)]
pub(crate) struct ToolCallDto {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

/// Сериализует успешный результат в JSON `ToolResult`.
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
        "metadata": add_tool_name(metadata, tool_name),
    });
    RResult::ROk(RString::from(result.to_string()))
}

/// Сериализует ошибку как `ToolResult` с `ok=false`.
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

/// Plugin-level error (не тот, что ToolResult.error). Используется когда
/// tool не смог вообще выполниться — невалидный JSON call, крэш и т.п.
pub(crate) fn plugin_error(message: String) -> RResult<RString, PluginToolError> {
    RResult::RErr(PluginToolError::new(message))
}

fn add_tool_name(mut metadata: Value, tool_name: &str) -> Value {
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("tool".to_owned(), json!(tool_name));
        Value::Object(obj.clone())
    } else {
        json!({ "tool": tool_name })
    }
}

/// Парсит `ToolCall` из JSON-строки.
pub(crate) fn parse_call(call_json: &str) -> Result<ToolCallDto, String> {
    serde_json::from_str(call_json).map_err(|e| format!("failed to parse ToolCall: {e}"))
}

/// Проверяет что `path` лежит внутри `cwd`. Возвращает canonical path.
///
/// Sync-версия workspace_path из builtin, через `std::fs::canonicalize`.
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

/// Как `workspace_path`, но для операций создания/записи: родительская
/// директория должна существовать и быть внутри workspace, сам файл может
/// не существовать.
pub(crate) fn workspace_path_for_write(cwd: &Path, path: &Path) -> Result<PathBuf, String> {
    let base = std::fs::canonicalize(cwd)
        .map_err(|e| format!("failed to canonicalize cwd {}: {e}", cwd.display()))?;
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let parent = target
        .parent()
        .ok_or_else(|| format!("no parent for {}", target.display()))?;
    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|e| format!("failed to canonicalize parent {}: {e}", parent.display()))?;
    if !canonical_parent.starts_with(&base) {
        return Err(format!("path escapes workspace: {}", path.display()));
    }
    let file_name = target
        .file_name()
        .ok_or_else(|| format!("no file name in {}", target.display()))?;
    Ok(canonical_parent.join(file_name))
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
