//! Git tools plugin: read-only git_status and git_diff.
//!
//! These tools run fixed git subcommands in the current workspace. They are
//! plugin tools, not core runtime behavior, so coding profiles can opt into
//! them through `tools.enabled` and policy.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::{
    io::Read,
    path::{Component, Path},
    process::{Child, Command, ExitStatus, Stdio},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use agent_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginTool,
        PluginTool_TO, PluginToolError, PluginToolObject,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};

const TIMEOUT_MS: u64 = 10_000;
const DEFAULT_MAX_BYTES: usize = 64 * 1024;
const MAX_MAX_BYTES: usize = 200 * 1024;

struct GitStatusTool;
struct GitDiffTool;

impl PluginTool for GitStatusTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "git_status",
            "description": "Show the current git branch and working tree status in short format.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum output bytes to return. Defaults to 65536 and is capped at 204800."
                    }
                }
            },
            "safety": "ReadOnly",
            "timeout_ms": TIMEOUT_MS,
            "metadata": null
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        invoke_tool(call_json.as_str(), cwd.as_str(), GitCommand::Status)
    }
}

impl PluginTool for GitDiffTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "git_diff",
            "description": "Show a bounded git diff for the workspace, optionally scoped to staged changes or a relative path.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "cached": {
                        "type": "boolean",
                        "description": "When true, show staged changes with git diff --cached."
                    },
                    "stat": {
                        "type": "boolean",
                        "description": "When true, show --stat instead of patch text."
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional workspace-relative pathspec. Absolute paths and parent traversal are rejected."
                    },
                    "context_lines": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Unified diff context lines. Defaults to 3 and is capped at 20."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum output bytes to return. Defaults to 65536 and is capped at 204800."
                    }
                }
            },
            "safety": "ReadOnly",
            "timeout_ms": TIMEOUT_MS,
            "metadata": null
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        invoke_tool(call_json.as_str(), cwd.as_str(), GitCommand::Diff)
    }
}

#[derive(Debug, Deserialize)]
struct ToolCallDto {
    id: String,
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Clone, Copy)]
enum GitCommand {
    Status,
    Diff,
}

fn invoke_tool(
    call_json: &str,
    cwd: &str,
    command_kind: GitCommand,
) -> RResult<RString, PluginToolError> {
    let call = match parse_call(call_json) {
        Ok(call) => call,
        Err(error) => return RResult::RErr(PluginToolError::new(error)),
    };
    match invoke_impl(&call, cwd, command_kind) {
        Ok(result) => RResult::ROk(RString::from(result)),
        Err(error) => RResult::ROk(RString::from(
            tool_result(&call.id, &call.name, false, "", Some(error), json!({})).to_string(),
        )),
    }
}

fn parse_call(call_json: &str) -> Result<ToolCallDto, String> {
    serde_json::from_str(call_json).map_err(|e| format!("failed to parse ToolCall: {e}"))
}

fn invoke_impl(call: &ToolCallDto, cwd: &str, command_kind: GitCommand) -> Result<String, String> {
    let max_bytes = optional_usize(&call.args, "max_bytes")?
        .unwrap_or(DEFAULT_MAX_BYTES)
        .min(MAX_MAX_BYTES);
    let cwd_path = Path::new(cwd);
    if !cwd_path.exists() {
        return Err(format!("cwd does not exist: {}", cwd_path.display()));
    }

    let mut command = Command::new("git");
    command
        .current_dir(cwd_path)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "diff.external=",
            "-c",
            "diff.trustExitCode=false",
        ])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match command_kind {
        GitCommand::Status => {
            command.args(["status", "--short", "--branch"]);
        }
        GitCommand::Diff => {
            command.arg("diff");
            command.args(["--no-ext-diff", "--no-textconv"]);
            if call
                .args
                .get("cached")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                command.arg("--cached");
            }
            if call
                .args
                .get("stat")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                command.arg("--stat");
            } else {
                let context_lines = optional_usize(&call.args, "context_lines")?
                    .unwrap_or(3)
                    .min(20);
                command.arg(format!("--unified={context_lines}"));
            }
            if let Some(path) = call.args.get("path").and_then(Value::as_str) {
                validate_relative_pathspec(path)?;
                command.arg("--").arg(path);
            }
        }
    }

    let child = command
        .spawn()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    let (output, timed_out) =
        wait_with_timeout(child, Duration::from_millis(TIMEOUT_MS), max_bytes)
            .map_err(|e| format!("failed to wait for git: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout.bytes).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr.bytes).into_owned();
    let mut rendered = stdout.clone();
    if !stderr.is_empty() {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&stderr);
    }
    if rendered.is_empty() && output.status.success() {
        rendered = match command_kind {
            GitCommand::Status => "(clean working tree)".to_owned(),
            GitCommand::Diff => "(no diff)".to_owned(),
        };
    }

    let status = output.status.code();
    let ok = output.status.success() && !timed_out;
    let error = if timed_out {
        Some(format!("git command timed out after {TIMEOUT_MS}ms"))
    } else if ok {
        None
    } else {
        Some(match status {
            Some(code) => format!("git exited with code {code}"),
            None => "git terminated by signal".to_owned(),
        })
    };
    let metadata = json!({
        "status": status,
        "stdout_bytes": output.stdout.original_len,
        "stderr_bytes": output.stderr.original_len,
        "stdout_truncated": output.stdout.truncated,
        "stderr_truncated": output.stderr.truncated,
        "timed_out": timed_out,
        "timeout_ms": TIMEOUT_MS,
        "max_bytes": max_bytes,
    });

    Ok(tool_result(&call.id, &call.name, ok, &rendered, error, metadata).to_string())
}

fn tool_result(
    call_id: &str,
    tool_name: &str,
    ok: bool,
    output: &str,
    error: Option<String>,
    metadata: Value,
) -> Value {
    let mut metadata = metadata;
    if let Some(object) = metadata.as_object_mut() {
        object.insert("tool".to_owned(), json!(tool_name));
    }
    json!({
        "call_id": call_id,
        "ok": ok,
        "output": output,
        "content": [],
        "error": error,
        "metadata": metadata,
    })
}

fn optional_usize(args: &Value, key: &str) -> Result<Option<usize>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let Some(number) = value.as_u64() else {
        return Err(format!("'{key}' must be a positive integer"));
    };
    if number == 0 {
        return Err(format!("'{key}' must be greater than zero"));
    }
    Ok(Some(number as usize))
}

fn validate_relative_pathspec(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("path must not be empty".to_owned());
    }
    let path = Path::new(path);
    if path.is_absolute() {
        return Err("path must be workspace-relative".to_owned());
    }
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err("path must not contain parent traversal".to_owned());
        }
    }
    Ok(())
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    original_len: usize,
    truncated: bool,
}

struct GitOutput {
    status: ExitStatus,
    stdout: BoundedBuffer,
    stderr: BoundedBuffer,
}

fn wait_with_timeout(
    mut child: Child,
    timeout: Duration,
    max_bytes: usize,
) -> std::io::Result<(GitOutput, bool)> {
    let mut stdout = child.stdout.take().expect("stdout pipe");
    let mut stderr = child.stderr.take().expect("stderr pipe");
    let stdout_handle = std::thread::spawn(move || read_limited(&mut stdout, max_bytes));
    let stderr_handle = std::thread::spawn(move || read_limited(&mut stderr, max_bytes));
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = join_reader(stdout_handle)?;
    let stderr = join_reader(stderr_handle)?;
    Ok((
        GitOutput {
            status,
            stdout,
            stderr,
        },
        timed_out,
    ))
}

fn read_limited<R: Read>(reader: &mut R, max_bytes: usize) -> std::io::Result<BoundedBuffer> {
    let mut output = Vec::new();
    let mut original_len = 0usize;
    let mut buffer = [0u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        original_len += read;
        if output.len() < max_bytes {
            let remaining = max_bytes - output.len();
            output.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
    Ok(BoundedBuffer {
        bytes: output,
        original_len,
        truncated: original_len > max_bytes,
    })
}

fn join_reader(
    handle: JoinHandle<std::io::Result<BoundedBuffer>>,
) -> std::io::Result<BoundedBuffer> {
    match handle.join() {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::other("reader thread panicked")),
    }
}

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let status: PluginToolObject = PluginTool_TO::from_value(GitStatusTool, TD_Opaque);
    if let RResult::RErr(err) = registry.register_tool(status) {
        return RResult::RErr(err);
    }

    let diff: PluginToolObject = PluginTool_TO::from_value(GitDiffTool, TD_Opaque);
    if let RResult::RErr(err) = registry.register_tool(diff) {
        return RResult::RErr(err);
    }

    RResult::ROk(())
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("git-tools"),
        description: RStr::from_str("Read-only git status and diff tools"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoke(tool_name: &str, cwd: &Path, args: Value) -> Value {
        let command = if tool_name == "git_status" {
            GitCommand::Status
        } else {
            GitCommand::Diff
        };
        let call = ToolCallDto {
            id: "call_test".to_owned(),
            name: tool_name.to_owned(),
            args,
        };
        let result = invoke_impl(&call, cwd.to_str().expect("utf-8 cwd"), command)
            .expect("tool result json");
        serde_json::from_str(&result).expect("result json")
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn git_status_reports_modified_file() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("workspace");
        git(dir.path(), &["init"]);
        std::fs::write(dir.path().join("notes.txt"), "one\n").expect("write file");

        let result = invoke("git_status", dir.path(), json!({}));

        assert_eq!(result["ok"], true);
        assert!(result["output"].as_str().unwrap().contains("notes.txt"));
    }

    #[test]
    fn git_diff_supports_path_filter() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("workspace");
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        git(dir.path(), &["config", "user.name", "Test"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").expect("write a");
        std::fs::write(dir.path().join("b.txt"), "one\n").expect("write b");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "initial"]);
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").expect("modify a");
        std::fs::write(dir.path().join("b.txt"), "one\nthree\n").expect("modify b");

        let result = invoke("git_diff", dir.path(), json!({ "path": "a.txt" }));

        assert_eq!(result["ok"], true);
        let output = result["output"].as_str().unwrap();
        assert!(output.contains("a.txt"), "{output}");
        assert!(!output.contains("b.txt"), "{output}");
    }

    #[test]
    fn pathspec_rejects_parent_escape() {
        let error = validate_relative_pathspec("../outside.txt").expect_err("reject parent");
        assert!(error.contains("parent traversal"), "{error}");
    }
}
