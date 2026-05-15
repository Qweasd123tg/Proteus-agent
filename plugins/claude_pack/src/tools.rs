use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::{Value, json};

use crate::util::{
    err_result, ok_result, optional_positive_usize, parse_call, plugin_error, required_string,
    workspace_path, workspace_path_for_write,
};

const SHELL_TIMEOUT_MS: u64 = 30_000;
const OUTPUT_LIMIT_BYTES: usize = 64 * 1024;

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
            "description": "Read a UTF-8 file from the current workspace. Prefer this over Bash cat/head/tail/sed. Use start_line and limit when you already know the relevant range. Results can include 1-based line numbers for precise edits.",
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
            "timeout_ms": 5000,
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
        let target = match workspace_path(Path::new(cwd.as_str()), Path::new(path)) {
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
                    "path is a directory; use Glob or Bash ls: {}",
                    target.display()
                ),
            );
        }
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
            "timeout_ms": 5000,
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
            "timeout_ms": 5000,
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
            "description": "Search file contents using ripgrep. Always prefer this over Bash grep/rg for workspace content search because it is permission-aware and returns reviewable output.",
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
            "timeout_ms": 15000,
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
        let lines = match run_lines_limited(command, max_results, Duration::from_secs(15)) {
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
            "description": "Find files by glob pattern in the current workspace. Prefer this over Bash find/ls for file discovery. Returns paths sorted by ripgrep's traversal order.",
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
            "timeout_ms": 15000,
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
        let lines = match run_lines_limited(command, max_results, Duration::from_secs(15)) {
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

impl PluginTool for BashTool {
    fn spec_json(&self) -> RString {
        RString::from(json!({
            "name": "Bash",
            "description": "Run a bash-compatible shell command in the workspace. Reserve this for terminal/system operations. Do not use it for reading, writing, editing, finding files, or searching contents when Read, Write, Edit, Glob, or Grep can do the job. Be careful with destructive git/filesystem commands.",
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
    let child = spawn_shell(command, cwd).map_err(|e| format!("failed to spawn shell: {e}"))?;
    let (output, timed_out) = wait_with_timeout(child, Duration::from_millis(SHELL_TIMEOUT_MS))
        .map_err(|e| format!("failed to wait for shell: {e}"))?;
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
        }
    })
    .to_string())
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
