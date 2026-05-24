use std::{
    fs,
    io::{BufRead, BufReader, ErrorKind, Read},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::util::{
    err_result, ok_result, optional_positive_usize, parse_call, plugin_error, required_string,
    workspace_path, workspace_path_for_write,
};

const SHELL_TIMEOUT_MS: u64 = 600_000;
const FILE_TOOL_TIMEOUT_MS: u64 = 60_000;
const SEARCH_TOOL_TIMEOUT_MS: u64 = 60_000;
const OUTPUT_LIMIT_BYTES: usize = 64 * 1024;
const EXTERNAL_TERMINAL_ENV: &str = "AGENT_SHELL_EXTERNAL_TERMINAL";
const EXTERNAL_TERMINAL_DBUS_ADDRESS_ENV: &str = "AGENT_SHELL_EXTERNAL_DBUS_ADDRESS";
const PTYXIS_TERMINAL: &str = "ptyxis";

pub(crate) struct ReadTool;
pub(crate) struct WriteTool;
pub(crate) struct EditTool;
pub(crate) struct GrepTool;
pub(crate) struct GlobTool;
pub(crate) struct BashTool;
pub(crate) struct TodoWriteTool;

impl PluginTool for ReadTool {
    fn spec_json(&self) -> RString {
        RString::from(json!({
            "name": "Read",
            "description": "Read a UTF-8 text file from the current workspace. Prefer this over Bash cat/head/tail/sed. Use start_line and limit when you already know the relevant range. Results can include 1-based line numbers for precise edits. If a path is not found, do not guess repeated variants; make at most one focused Glob/Grep recovery attempt, then ask for the exact path or report that it was not found.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Path to read, relative to workspace or absolute inside workspace." },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "limit": { "type": "integer", "minimum": 1 },
                    "line_numbers": { "type": "boolean", "description": "Prefix lines with 1-based line numbers." }
                },
                "required": ["file_path"]
            },
            "safety": "ReadOnly",
            "timeout_ms": FILE_TOOL_TIMEOUT_MS,
            "metadata": { "claude_pack": true, "alias_for": "read_file" }
        }).to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(call) => call,
            Err(e) => return plugin_error(e),
        };
        let path = match required_string(&call.args, "file_path", &call.name) {
            Ok(path) => path,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let cwd_path = Path::new(cwd.as_str());
        let target = match read_workspace_path(cwd_path, Path::new(path)) {
            Ok(path) => path,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let metadata = match std::fs::metadata(&target) {
            Ok(metadata) => metadata,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to inspect {}: {e}", target.display()),
                );
            }
        };
        if metadata.is_dir() {
            return err_result(
                &call.id,
                &call.name,
                format!(
                    "path is a directory, not a file: {}. Provide a concrete file path; do not run Bash to list it.",
                    target.display()
                ),
            );
        }
        let content = match std::fs::read_to_string(&target) {
            Ok(content) => content,
            Err(e) if e.kind() == ErrorKind::InvalidData => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!(
                        "file is not UTF-8 text: {}. Read in claude_pack only handles text files; do not search for alternate paths unless the user asked for a different file.",
                        target.display()
                    ),
                );
            }
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to read {}: {e}", target.display()),
                );
            }
        };
        let start_line = match optional_positive_usize(&call.args, "start_line", &call.name) {
            Ok(value) => value.unwrap_or(1),
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let limit = match optional_positive_usize(&call.args, "limit", &call.name) {
            Ok(value) => value.unwrap_or(usize::MAX),
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let line_numbers = call
            .args
            .get("line_numbers")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let total_lines = content.lines().count();
        let mut returned = 0usize;
        let mut lines = Vec::new();
        for (index, line) in content
            .lines()
            .enumerate()
            .skip(start_line.saturating_sub(1))
            .take(limit)
        {
            returned += 1;
            if line_numbers {
                lines.push(format!("{}\t{}", index + 1, line));
            } else {
                lines.push(line.to_owned());
            }
        }
        ok_result(
            &call.id,
            &call.name,
            lines.join("\n"),
            json!({
                "path": target.display().to_string(),
                "start_line": start_line,
                "returned_lines": returned,
                "total_lines": total_lines,
                "truncated": start_line.saturating_sub(1) + returned < total_lines,
            }),
        )
    }
}

impl PluginTool for WriteTool {
    fn spec_json(&self) -> RString {
        RString::from(json!({
            "name": "Write",
            "description": "Create or fully overwrite a UTF-8 file inside the current workspace. Creates missing parent directories. Prefer Edit for modifying existing files. If overwriting an existing file, read it first unless the user explicitly requested a full rewrite.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["file_path", "content"]
            },
            "safety": "WritesFiles",
            "timeout_ms": FILE_TOOL_TIMEOUT_MS,
            "metadata": { "claude_pack": true, "alias_for": "write_file" }
        }).to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(call) => call,
            Err(e) => return plugin_error(e),
        };
        let path = match required_string(&call.args, "file_path", &call.name) {
            Ok(path) => path,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let content = match required_string(&call.args, "content", &call.name) {
            Ok(content) => content,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let target = match workspace_path_for_write(Path::new(cwd.as_str()), Path::new(path)) {
            Ok(path) => path,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let existed = target.exists();
        if let Err(e) = std::fs::write(&target, content) {
            return err_result(
                &call.id,
                &call.name,
                format!("failed to write {}: {e}", target.display()),
            );
        }
        ok_result(
            &call.id,
            &call.name,
            format!("Wrote {} bytes to {}", content.len(), target.display()),
            json!({
                "path": target.display().to_string(),
                "bytes_written": content.len(),
                "existed": existed,
            }),
        )
    }
}

impl PluginTool for EditTool {
    fn spec_json(&self) -> RString {
        RString::from(json!({
            "name": "Edit",
            "description": "Perform exact string replacements in an existing UTF-8 file inside the workspace. Read the file first, preserve exact indentation from Read output, and use the smallest old_string that is clearly unique. Prefer this over Bash sed/awk or echo redirection.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "file_path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean", "description": "Replace every exact occurrence. Defaults to false and requires old_string to be unique." }
                },
                "required": ["file_path", "old_string", "new_string"]
            },
            "safety": "WritesFiles",
            "timeout_ms": FILE_TOOL_TIMEOUT_MS,
            "metadata": { "claude_pack": true, "alias_for": "apply_patch" }
        }).to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(call) => call,
            Err(e) => return plugin_error(e),
        };
        let path = match required_string(&call.args, "file_path", &call.name) {
            Ok(path) => path,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let old = match required_string(&call.args, "old_string", &call.name) {
            Ok(value) => value,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let new = match required_string(&call.args, "new_string", &call.name) {
            Ok(value) => value,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        if old.is_empty() {
            return err_result(
                &call.id,
                &call.name,
                "old_string must not be empty".to_owned(),
            );
        }
        let replace_all = call
            .args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let target = match workspace_path(Path::new(cwd.as_str()), Path::new(path)) {
            Ok(path) => path,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let content = match std::fs::read_to_string(&target) {
            Ok(content) => content,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to read {}: {e}", target.display()),
                );
            }
        };
        let occurrences = content.matches(old).count();
        if occurrences == 0 {
            return err_result(
                &call.id,
                &call.name,
                "old_string was not found in file".to_owned(),
            );
        }
        if occurrences > 1 && !replace_all {
            return err_result(
                &call.id,
                &call.name,
                format!(
                    "old_string is not unique ({occurrences} matches); provide more context or set replace_all"
                ),
            );
        }
        let updated = if replace_all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        };
        if let Err(e) = std::fs::write(&target, updated) {
            return err_result(
                &call.id,
                &call.name,
                format!("failed to write {}: {e}", target.display()),
            );
        }
        ok_result(
            &call.id,
            &call.name,
            format!(
                "Edited {} ({occurrences} replacement{})",
                target.display(),
                if occurrences == 1 { "" } else { "s" }
            ),
            json!({
                "path": target.display().to_string(),
                "replacements": if replace_all { occurrences } else { 1 },
            }),
        )
    }
}

impl PluginTool for GrepTool {
    fn spec_json(&self) -> RString {
        RString::from(json!({
            "name": "Grep",
            "description": "Search file contents using ripgrep. Always prefer this over Bash grep/rg for workspace content search because it is permission-aware and returns reviewable output. Do not chain broad searches after no matches; make one focused refinement, then ask or report that nothing was found.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Ripgrep regex pattern." },
                    "path": { "type": "string", "description": "Workspace-relative directory/file to search. Defaults to root." },
                    "glob": { "type": "string", "description": "Optional glob filter such as \"*.rs\" or \"**/*.ts\"." },
                    "max_results": { "type": "integer", "minimum": 1 }
                },
                "required": ["pattern"]
            },
            "safety": "ReadOnly",
            "timeout_ms": SEARCH_TOOL_TIMEOUT_MS,
            "metadata": { "claude_pack": true, "alias_for": "grep" }
        }).to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(call) => call,
            Err(e) => return plugin_error(e),
        };
        let pattern = match required_string(&call.args, "pattern", &call.name) {
            Ok(pattern) => pattern,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let path = call.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let target = match workspace_path(Path::new(cwd.as_str()), Path::new(path)) {
            Ok(path) => path,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let max_results = match optional_positive_usize(&call.args, "max_results", &call.name) {
            Ok(value) => value.unwrap_or(50),
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
            .arg("1M");
        if let Some(glob) = call.args.get("glob").and_then(Value::as_str) {
            command.arg("--glob").arg(glob);
        }
        command
            .arg("--")
            .arg(pattern)
            .arg(&target)
            .stdin(Stdio::null());
        let lines = match run_lines_limited(
            command,
            max_results,
            Duration::from_millis(SEARCH_TOOL_TIMEOUT_MS),
        ) {
            Ok(lines) => lines,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to run ripgrep: {e} (is 'rg' installed?)"),
                );
            }
        };
        ok_result(
            &call.id,
            &call.name,
            if lines.is_empty() {
                "(no matches)".to_owned()
            } else {
                lines.join("\n")
            },
            json!({
                "pattern": pattern,
                "path": target.display().to_string(),
                "match_count": lines.len(),
                "max_results": max_results,
                "truncated": lines.len() >= max_results,
            }),
        )
    }
}

impl PluginTool for GlobTool {
    fn spec_json(&self) -> RString {
        RString::from(json!({
            "name": "Glob",
            "description": "Find files by glob pattern in the current workspace. Prefer this over Bash find/ls for file discovery. Returns paths sorted by ripgrep's traversal order. Do not chain broad glob variants after no matches; make one focused refinement, then ask or report that nothing was found.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern, e.g. \"**/*.rs\" or \"src/**/*.ts\"." },
                    "path": { "type": "string", "description": "Workspace-relative search root. Defaults to root." },
                    "max_results": { "type": "integer", "minimum": 1 }
                },
                "required": ["pattern"]
            },
            "safety": "ReadOnly",
            "timeout_ms": SEARCH_TOOL_TIMEOUT_MS,
            "metadata": { "claude_pack": true }
        }).to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(call) => call,
            Err(e) => return plugin_error(e),
        };
        let pattern = match required_string(&call.args, "pattern", &call.name) {
            Ok(pattern) => pattern,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let root = call.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let target = match workspace_path(Path::new(cwd.as_str()), Path::new(root)) {
            Ok(path) => path,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let max_results = match optional_positive_usize(&call.args, "max_results", &call.name) {
            Ok(value) => value.unwrap_or(100),
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let mut command = Command::new("rg");
        command
            .arg("--files")
            .arg("--glob")
            .arg(pattern)
            .arg(&target)
            .stdin(Stdio::null());
        let lines = match run_lines_limited(
            command,
            max_results,
            Duration::from_millis(SEARCH_TOOL_TIMEOUT_MS),
        ) {
            Ok(lines) => lines,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to run ripgrep files search: {e} (is 'rg' installed?)"),
                );
            }
        };
        ok_result(
            &call.id,
            &call.name,
            if lines.is_empty() {
                "(no matches)".to_owned()
            } else {
                lines.join("\n")
            },
            json!({
                "pattern": pattern,
                "path": target.display().to_string(),
                "match_count": lines.len(),
                "max_results": max_results,
                "truncated": lines.len() >= max_results,
            }),
        )
    }
}

fn read_workspace_path(cwd: &Path, path: &Path) -> Result<PathBuf, String> {
    let base = std::fs::canonicalize(cwd)
        .map_err(|e| format!("failed to canonicalize cwd {}: {e}", cwd.display()))?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };

    if candidate.exists() {
        let canonical = std::fs::canonicalize(&candidate)
            .map_err(|e| format!("failed to canonicalize {}: {e}", candidate.display()))?;
        if !canonical.starts_with(&base) {
            return Err(format!("path escapes workspace: {}", path.display()));
        }
        return Ok(canonical);
    }

    let safe_missing = missing_candidate_inside_workspace(&base, path)?;
    Err(missing_file_message(&base, &safe_missing))
}

fn missing_candidate_inside_workspace(base: &Path, path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        let relative = path
            .strip_prefix(base)
            .map_err(|_| format!("path escapes workspace: {}", path.display()))?;
        return Ok(base.join(safe_relative_read_path(relative)?));
    }
    Ok(base.join(safe_relative_read_path(path)?))
}

fn safe_relative_read_path(path: &Path) -> Result<PathBuf, String> {
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => safe.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(format!("path escapes workspace: {}", path.display()));
            }
        }
    }
    if safe.as_os_str().is_empty() {
        return Err(format!("no file name in {}", path.display()));
    }
    Ok(safe)
}

fn missing_file_message(base: &Path, target: &Path) -> String {
    let mut message = format!(
        "file does not exist: {}. Current workspace: {}.",
        target.display(),
        base.display()
    );
    if let Some(suggestion) = suggest_similar_file(target) {
        message.push_str(&format!(" Did you mean {}?", suggestion.display()));
    }
    message.push_str(
        " Do not guess repeated path variants or use Bash for file discovery; make at most one focused Glob/Grep recovery attempt if necessary.",
    );
    message
}

fn suggest_similar_file(target: &Path) -> Option<PathBuf> {
    let parent = target.parent()?;
    let requested_name = target.file_name()?.to_str()?;
    let requested_stem = target.file_stem()?.to_str()?;
    let entries = std::fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();

    entries
        .iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(requested_name))
        })
        .cloned()
        .or_else(|| {
            entries.into_iter().find(|path| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem == requested_stem)
            })
        })
}

impl PluginTool for BashTool {
    fn spec_json(&self) -> RString {
        RString::from(json!({
            "name": "Bash",
            "description": "Run a bash-compatible shell command in the workspace. When used from agent-tui the command opens in a visible tab of its dedicated Ptyxis tool window; tabs remain open after completion. Reserve this for terminal/system operations. Do not use it for reading, writing, editing, finding files, or searching contents when Read, Write, Edit, Glob, or Grep can do the job. Be careful with destructive git/filesystem commands.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "description": { "type": "string", "description": "Brief active-voice description of what the command does." }
                },
                "required": ["command"]
            },
            "safety": "RunsCommands",
            "timeout_ms": SHELL_TIMEOUT_MS,
            "metadata": { "claude_pack": true, "alias_for": "shell" }
        }).to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        match invoke_bash(call_json.as_str(), cwd.as_str()) {
            Ok(result) => RResult::ROk(RString::from(result)),
            Err(e) => RResult::RErr(PluginToolError::new(e)),
        }
    }
}

impl PluginTool for TodoWriteTool {
    fn spec_json(&self) -> RString {
        RString::from(json!({
            "name": "TodoWrite",
            "description": "Create or update a structured todo list for the current coding task. Use proactively for tasks with 3+ steps, multiple files, explicit user checklists, or non-trivial debugging. Keep exactly one task in_progress when actively working; do not mark failing or partial work completed.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": { "type": "string" },
                                "active_form": { "type": "string" },
                                "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                            },
                            "required": ["content", "active_form", "status"]
                        }
                    }
                },
                "required": ["todos"]
            },
            "safety": "ReadOnly",
            "timeout_ms": 1000,
            "metadata": { "claude_pack": true }
        }).to_string())
    }

    fn invoke_json(&self, call_json: RString, _cwd: RString) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(call) => call,
            Err(e) => return plugin_error(e),
        };
        let todos = match call.args.get("todos").and_then(Value::as_array) {
            Some(todos) => todos,
            None => {
                return err_result(
                    &call.id,
                    &call.name,
                    "TodoWrite requires array arg 'todos'".to_owned(),
                );
            }
        };
        let mut in_progress = 0usize;
        let mut lines = Vec::new();
        for (index, todo) in todos.iter().enumerate() {
            let content = todo.get("content").and_then(Value::as_str).unwrap_or("");
            let active = todo
                .get("active_form")
                .and_then(Value::as_str)
                .unwrap_or(content);
            let status = todo
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            if !matches!(status, "pending" | "in_progress" | "completed") {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("todo {} has invalid status '{status}'", index + 1),
                );
            }
            if status == "in_progress" {
                in_progress += 1;
            }
            let marker = match status {
                "completed" => "x",
                "in_progress" => ">",
                _ => " ",
            };
            lines.push(format!(
                "[{marker}] {}{}",
                content,
                if status == "in_progress" {
                    format!(" ({active})")
                } else {
                    String::new()
                }
            ));
        }
        ok_result(
            &call.id,
            &call.name,
            if lines.is_empty() {
                "(todo list cleared)".to_owned()
            } else {
                lines.join("\n")
            },
            json!({
                "todo_count": todos.len(),
                "in_progress_count": in_progress,
                "warning": if in_progress > 1 { "more than one in_progress todo" } else { "" },
            }),
        )
    }
}

fn invoke_bash(call_json: &str, cwd: &str) -> Result<String, String> {
    let call = parse_call(call_json)?;
    let command = required_string(&call.args, "command", &call.name)?;
    let (output, timed_out, external_terminal) = if should_use_ptyxis() {
        let (output, timed_out) =
            run_in_ptyxis(command, cwd, Duration::from_millis(SHELL_TIMEOUT_MS))?;
        (output, timed_out, Some(PTYXIS_TERMINAL))
    } else {
        let child = spawn_shell(command, cwd).map_err(|e| format!("failed to spawn shell: {e}"))?;
        let (output, timed_out) = wait_with_timeout(child, Duration::from_millis(SHELL_TIMEOUT_MS))
            .map_err(|e| format!("failed to wait for shell: {e}"))?;
        (output, timed_out, None)
    };
    let stdout = String::from_utf8_lossy(&output.stdout.bytes).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr.bytes).into_owned();
    let mut rendered = stdout;
    if !stderr.is_empty() {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&stderr);
    }
    let status = output.status.code();
    let success = output.status.success();
    let error = if timed_out {
        Some(format!("process timed out after {SHELL_TIMEOUT_MS}ms"))
    } else if !success {
        Some(match status {
            Some(code) => format!("process exited with code {code}"),
            None => "process terminated by signal".to_owned(),
        })
    } else {
        None
    };
    Ok(json!({
        "call_id": call.id,
        "ok": success,
        "output": rendered,
        "content": [],
        "error": error,
        "metadata": {
            "tool": call.name,
            "status": status,
            "stdout_bytes": output.stdout.original_len,
            "stderr_bytes": output.stderr.original_len,
            "stdout_truncated": output.stdout.truncated,
            "stderr_truncated": output.stderr.truncated,
            "timed_out": timed_out,
            "timeout_ms": SHELL_TIMEOUT_MS,
            "external_terminal": external_terminal,
        }
    })
    .to_string())
}

fn should_use_ptyxis() -> bool {
    std::env::var(EXTERNAL_TERMINAL_ENV)
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case(PTYXIS_TERMINAL))
}

fn run_in_ptyxis(
    command: &str,
    cwd: &str,
    timeout: Duration,
) -> Result<(ShellOutput, bool), String> {
    let capture_dir = tempfile::Builder::new()
        .prefix("agent-bash-ptyxis-")
        .tempdir()
        .map_err(|error| format!("failed to create Ptyxis capture directory: {error}"))?;
    let paths = PtyxisCapturePaths::new(capture_dir.path());
    fs::write(&paths.wrapper, ptyxis_wrapper_script())
        .map_err(|error| format!("failed to write Ptyxis wrapper: {error}"))?;
    spawn_ptyxis(command, cwd, &paths)?;
    wait_for_ptyxis_result(capture_dir, paths, timeout)
}

struct PtyxisCapturePaths {
    wrapper: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
    status: PathBuf,
    pid: PathBuf,
}

impl PtyxisCapturePaths {
    fn new(dir: &Path) -> Self {
        Self {
            wrapper: dir.join("run.sh"),
            stdout: dir.join("stdout.log"),
            stderr: dir.join("stderr.log"),
            status: dir.join("status"),
            pid: dir.join("pid"),
        }
    }
}

fn ptyxis_wrapper_script() -> &'static str {
    r#"#!/usr/bin/env bash
set +e
command_text="$1"
stdout_path="$2"
stderr_path="$3"
status_path="$4"
pid_path="$5"
command_pid=""
finish() {
    local status="$1"
    trap - EXIT HUP INT TERM
    if [ -n "$command_pid" ]; then
        kill -- "-$command_pid" 2>/dev/null || true
    fi
    printf '%s\n' "$status" > "$status_path"
    exit "$status"
}
trap 'finish 130' HUP INT TERM
setsid sh -lc "$command_text" > >(tee "$stdout_path") 2> >(tee "$stderr_path" >&2) &
command_pid=$!
printf '%s\n' "$command_pid" > "$pid_path"
wait "$command_pid"
status=$?
trap - HUP INT TERM
printf '%s\n' "$status" > "$status_path"
printf '\n[agent] command finished with exit code %s; this tab remains open.\n' "$status"
exec bash --noprofile --norc -i
"#
}

fn spawn_ptyxis(command: &str, cwd: &str, paths: &PtyxisCapturePaths) -> Result<(), String> {
    let mut launcher = Command::new(PTYXIS_TERMINAL);
    if let Some(address) = std::env::var_os(EXTERNAL_TERMINAL_DBUS_ADDRESS_ENV) {
        launcher.env("DBUS_SESSION_BUS_ADDRESS", address);
    }
    let mut child = launcher
        .arg("--tab")
        .arg("--working-directory")
        .arg(cwd)
        .arg("--title")
        .arg(format!("agent Bash · {}", command_summary(command)))
        .arg("--execute")
        .arg(ptyxis_execute_command(
            command,
            paths,
            std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(),
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to open Ptyxis terminal: {error}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn ptyxis_execute_command(
    command: &str,
    paths: &PtyxisCapturePaths,
    original_dbus: Option<&str>,
) -> String {
    let mut execute = match original_dbus {
        Some(address) => format!("env DBUS_SESSION_BUS_ADDRESS={} ", shell_quote(address)),
        None => "env -u DBUS_SESSION_BUS_ADDRESS ".to_owned(),
    };
    execute.push_str("bash ");
    let arguments = [
        paths.wrapper.display().to_string(),
        command.to_owned(),
        paths.stdout.display().to_string(),
        paths.stderr.display().to_string(),
        paths.status.display().to_string(),
        paths.pid.display().to_string(),
    ];
    for argument in &arguments {
        execute.push_str(&shell_quote(argument));
        execute.push(' ');
    }
    execute.pop();
    execute
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn command_summary(command: &str) -> String {
    const MAX_TITLE_CHARS: usize = 60;
    let line = command.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= MAX_TITLE_CHARS {
        return line.to_owned();
    }
    let mut result = line.chars().take(MAX_TITLE_CHARS - 1).collect::<String>();
    result.push('…');
    result
}

fn wait_for_ptyxis_result(
    _capture_dir: TempDir,
    paths: PtyxisCapturePaths,
    timeout: Duration,
) -> Result<(ShellOutput, bool), String> {
    let started = Instant::now();
    loop {
        if let Ok(status_text) = fs::read_to_string(&paths.status) {
            let code = status_text
                .trim()
                .parse::<i32>()
                .map_err(|error| format!("failed to parse Ptyxis status: {error}"))?;
            return Ok((read_ptyxis_output(&paths, code)?, false));
        }
        if started.elapsed() >= timeout {
            kill_ptyxis_command(&paths.pid);
            return Ok((read_ptyxis_output(&paths, 124)?, true));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn read_ptyxis_output(paths: &PtyxisCapturePaths, code: i32) -> Result<ShellOutput, String> {
    Ok(ShellOutput {
        status: exit_status_from_code(code),
        stdout: read_bounded_file(&paths.stdout)?,
        stderr: read_bounded_file(&paths.stderr)?,
    })
}

fn read_bounded_file(path: &Path) -> Result<BoundedBuffer, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.to_string()),
    };
    let original_len = bytes.len();
    Ok(BoundedBuffer {
        bytes: bytes.into_iter().take(OUTPUT_LIMIT_BYTES).collect(),
        original_len,
        truncated: original_len > OUTPUT_LIMIT_BYTES,
    })
}

#[cfg(unix)]
fn kill_ptyxis_command(pid_path: &Path) {
    let Ok(pid_text) = fs::read_to_string(pid_path) else {
        return;
    };
    let Ok(pgid) = pid_text.trim().parse::<i32>() else {
        return;
    };
    unsafe {
        let _ = libc::kill(-pgid, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_ptyxis_command(_pid_path: &Path) {}

#[cfg(unix)]
fn exit_status_from_code(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;

    ExitStatus::from_raw(code << 8)
}

#[cfg(windows)]
fn exit_status_from_code(code: i32) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;

    ExitStatus::from_raw(code as u32)
}

fn spawn_shell(command: &str, cwd: &str) -> std::io::Result<Child> {
    let mut command_builder = Command::new("sh");
    command_builder
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        command_builder.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    command_builder.spawn()
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    original_len: usize,
    truncated: bool,
}

struct ShellOutput {
    status: ExitStatus,
    stdout: BoundedBuffer,
    stderr: BoundedBuffer,
}

fn wait_with_timeout(mut child: Child, timeout: Duration) -> std::io::Result<(ShellOutput, bool)> {
    let stdout_reader = spawn_bounded_reader(child.stdout.take());
    let stderr_reader = spawn_bounded_reader(child.stderr.take());
    let started = Instant::now();
    let (status, timed_out) = loop {
        if child.try_wait()?.is_some() {
            break (child.wait()?, false);
        }
        if started.elapsed() >= timeout {
            kill_child_tree(&mut child);
            break (child.wait()?, true);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok((
        ShellOutput {
            status,
            stdout: join_reader(stdout_reader)?,
            stderr: join_reader(stderr_reader)?,
        },
        timed_out,
    ))
}

fn spawn_bounded_reader<R>(reader: Option<R>) -> JoinHandle<std::io::Result<BoundedBuffer>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let Some(mut reader) = reader else {
            return Ok(BoundedBuffer {
                bytes: Vec::new(),
                original_len: 0,
                truncated: false,
            });
        };
        let mut bytes = Vec::new();
        let mut original_len = 0usize;
        let mut buf = [0u8; 8192];
        loop {
            let read = reader.read(&mut buf)?;
            if read == 0 {
                break;
            }
            original_len += read;
            if bytes.len() < OUTPUT_LIMIT_BYTES {
                let remaining = OUTPUT_LIMIT_BYTES - bytes.len();
                bytes.extend_from_slice(&buf[..read.min(remaining)]);
            }
        }
        Ok(BoundedBuffer {
            bytes,
            original_len,
            truncated: original_len > OUTPUT_LIMIT_BYTES,
        })
    })
}

fn join_reader(
    handle: JoinHandle<std::io::Result<BoundedBuffer>>,
) -> std::io::Result<BoundedBuffer> {
    handle
        .join()
        .map_err(|_| std::io::Error::other("shell output reader thread panicked"))?
}

#[cfg(unix)]
fn kill_child_tree(child: &mut Child) {
    let pgid = child.id() as i32;
    unsafe {
        let _ = libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_child_tree(child: &mut Child) {
    let _ = child.kill();
}

fn run_lines_limited(
    mut command: Command,
    max_results: usize,
    timeout: Duration,
) -> std::io::Result<Vec<String>> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("failed to open command stdout"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut lines = Vec::new();
        for line in reader.lines() {
            let line = line?;
            lines.push(line);
            if lines.len() >= max_results {
                break;
            }
        }
        let _ = tx.send(std::io::Result::Ok(lines));
        std::io::Result::Ok(())
    });
    let started = Instant::now();
    loop {
        match rx.try_recv() {
            Ok(lines) => {
                let _ = child.kill();
                let _ = child.wait();
                return lines;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::other("stdout reader stopped"));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if let Some(_status) = child.try_wait()? {
            return rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap_or_else(|_| Ok(Vec::new()));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "command timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[allow(dead_code)]
fn relative_display(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_contracts::abi_stable::std_types::RResult;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "claude-pack-tools-test-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    fn invoke_read(cwd: &Path, file_path: &str) -> Value {
        let call = json!({
            "id": "call-1",
            "name": "Read",
            "args": { "file_path": file_path }
        });
        match ReadTool.invoke_json(
            RString::from(call.to_string()),
            RString::from(cwd.display().to_string()),
        ) {
            RResult::ROk(result) => serde_json::from_str(result.as_str()).expect("ToolResult json"),
            RResult::RErr(error) => panic!("plugin error: {}", error.message),
        }
    }

    fn invoke_bash_tool(cwd: &Path, command: &str) -> Value {
        let call = json!({
            "id": "call-1",
            "name": "Bash",
            "args": { "command": command }
        });
        match BashTool.invoke_json(
            RString::from(call.to_string()),
            RString::from(cwd.display().to_string()),
        ) {
            RResult::ROk(result) => serde_json::from_str(result.as_str()).expect("ToolResult json"),
            RResult::RErr(error) => panic!("plugin error: {}", error.message),
        }
    }

    fn spec<T: PluginTool>(tool: &T) -> Value {
        serde_json::from_str(tool.spec_json().as_str()).expect("tool spec json")
    }

    #[test]
    fn tool_specs_allow_real_workspace_latency() {
        assert_eq!(spec(&ReadTool)["timeout_ms"], FILE_TOOL_TIMEOUT_MS);
        assert_eq!(spec(&WriteTool)["timeout_ms"], FILE_TOOL_TIMEOUT_MS);
        assert_eq!(spec(&EditTool)["timeout_ms"], FILE_TOOL_TIMEOUT_MS);
        assert_eq!(spec(&GrepTool)["timeout_ms"], SEARCH_TOOL_TIMEOUT_MS);
        assert_eq!(spec(&GlobTool)["timeout_ms"], SEARCH_TOOL_TIMEOUT_MS);
        assert_eq!(spec(&BashTool)["timeout_ms"], SHELL_TIMEOUT_MS);
        assert!(FILE_TOOL_TIMEOUT_MS >= 60_000);
        assert!(SEARCH_TOOL_TIMEOUT_MS >= 60_000);
        assert!(SHELL_TIMEOUT_MS >= 600_000);
    }

    #[test]
    fn read_directory_error_does_not_suggest_bash_or_glob_fallback() {
        let workspace = temp_workspace();
        std::fs::create_dir_all(workspace.join("src")).expect("create src");

        let result = invoke_read(&workspace, "src");
        let error = result["error"].as_str().expect("error string");
        assert!(!result["ok"].as_bool().expect("ok"));
        assert!(error.contains("directory, not a file"));
        assert!(!error.contains("Bash ls"));
        assert!(!error.contains("use Glob"));

        std::fs::remove_dir_all(workspace).expect("cleanup temp workspace");
    }

    #[test]
    fn read_missing_file_gives_cwd_and_did_you_mean_without_bash_discovery() {
        let workspace = temp_workspace();
        std::fs::create_dir_all(workspace.join("src")).expect("create src");
        std::fs::write(workspace.join("src/main.ts"), "fn main() {}\n").expect("write file");

        let result = invoke_read(&workspace, "src/main.rs");
        let error = result["error"].as_str().expect("error string");
        assert!(!result["ok"].as_bool().expect("ok"));
        assert!(error.contains("file does not exist"));
        assert!(error.contains("Current workspace"));
        assert!(error.contains("Did you mean"));
        assert!(error.contains("src/main.ts"));
        assert!(!error.contains("Bash ls"));

        std::fs::remove_dir_all(workspace).expect("cleanup temp workspace");
    }

    #[test]
    fn bash_failed_command_keeps_stderr_in_output() {
        let workspace = temp_workspace();

        let result = invoke_bash_tool(&workspace, "printf 'bad usage\\n' >&2; exit 2");

        assert!(!result["ok"].as_bool().expect("ok"));
        assert_eq!(result["output"], "bad usage\n");
        assert_eq!(result["error"], "process exited with code 2");
        assert_eq!(result["metadata"]["stderr_bytes"], 10);

        std::fs::remove_dir_all(workspace).expect("cleanup temp workspace");
    }

    #[test]
    fn ptyxis_command_title_is_single_line_and_bounded() {
        assert_eq!(command_summary("cargo test\nignored"), "cargo test");
        let title = command_summary(&"x".repeat(100));
        assert!(title.ends_with('…'));
        assert!(title.chars().count() <= 60);
    }

    #[test]
    fn ptyxis_wrapper_streams_output_records_status_and_stays_open() {
        let wrapper = ptyxis_wrapper_script();
        assert!(wrapper.contains("tee \"$stdout_path\""));
        assert!(wrapper.contains("trap 'finish 130' HUP INT TERM"));
        assert!(wrapper.contains("exec bash --noprofile --norc -i"));
    }

    #[test]
    fn ptyxis_execute_command_restores_desktop_bus_and_quotes_command() {
        let capture_dir = tempfile::tempdir().expect("capture dir");
        let paths = PtyxisCapturePaths::new(capture_dir.path());
        let execute =
            ptyxis_execute_command("printf '%s' done", &paths, Some("unix:path=/tmp/user bus"));
        assert!(
            execute.starts_with("env DBUS_SESSION_BUS_ADDRESS='unix:path=/tmp/user bus' bash ")
        );
        assert!(execute.contains("'printf '\"'\"'%s'\"'\"' done'"));
    }
}
