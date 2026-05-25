//! Shell tool как dylib-плагин.
//!
//! Регистрирует один tool `shell` через `PluginTool` ABI. Безопасность
//! `RunsCommands` — `PermissionMode::Auto` запретит без approval, `plan`
//! скроет вообще. Вынесен из ядра именно ради этого: shell — самая
//! рискованная вещь, логично делать её opt-in через плагин, а не
//! встраивать.
//!
//! Реализация держит stdout/stderr bounded и на Unix запускает shell в
//! отдельной process group, чтобы timeout мог остановить не только `sh`, но и
//! его дочерние процессы.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

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
use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use tempfile::TempDir;

/// Максимум stdout/stderr в килобайтах. Reader продолжает дренировать pipe
/// после лимита, но сохраняет только первые байты.
const OUTPUT_LIMIT_BYTES: usize = 64 * 1024;

/// Timeout на выполнение команды. Shell-команды часто запускают тесты,
/// сборки или генерацию артефактов, поэтому 30 секунд слишком агрессивны.
const TIMEOUT_MS: u64 = 600_000;
const EXTERNAL_TERMINAL_ENV: &str = "AGENT_SHELL_EXTERNAL_TERMINAL";
const EXTERNAL_TERMINAL_DBUS_ADDRESS_ENV: &str = "AGENT_SHELL_EXTERNAL_DBUS_ADDRESS";
const PTYXIS_TERMINAL: &str = "ptyxis";

struct ShellTool;

impl PluginTool for ShellTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "shell",
            "description": "Run a shell command in the current workspace (sh -lc). Agent TUI launches commands in a visible tab of its dedicated Ptyxis tool window; tabs remain open after completion. Safety: RunsCommands.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            },
            "safety": "RunsCommands",
            "timeout_ms": TIMEOUT_MS,
            "metadata": null
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        match invoke_impl(call_json.as_str(), cwd.as_str()) {
            Ok(result_json) => RResult::ROk(RString::from(result_json)),
            Err(error) => RResult::RErr(PluginToolError::new(format!("{error:#}"))),
        }
    }
}

fn invoke_impl(call_json: &str, cwd: &str) -> Result<String> {
    let call: Value =
        serde_json::from_str(call_json).with_context(|| "failed to parse ToolCall JSON")?;
    let call_id = call
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let command = call
        .get("args")
        .and_then(|args| args.get("command"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("shell requires string arg 'command'"))?;

    let (output, timed_out, external_terminal) = if should_use_ptyxis() {
        let (output, timed_out) = run_in_ptyxis(command, cwd, Duration::from_millis(TIMEOUT_MS))
            .with_context(|| "failed to run shell in Ptyxis")?;
        (output, timed_out, Some(PTYXIS_TERMINAL))
    } else {
        let child = spawn_shell(command, cwd).with_context(|| "failed to spawn shell")?;
        let (output, timed_out) = wait_with_timeout(child, Duration::from_millis(TIMEOUT_MS))
            .with_context(|| "failed to wait for shell")?;
        (output, timed_out, None)
    };

    let stdout = String::from_utf8_lossy(&output.stdout.bytes).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr.bytes).into_owned();
    let status = output.status.code();
    let success = output.status.success();

    let mut rendered = stdout.clone();
    if !stderr.is_empty() {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&stderr);
    }

    let error_msg = if timed_out {
        Some(format!("process timed out after {TIMEOUT_MS}ms"))
    } else if !success {
        Some(match status {
            Some(code) => format!("process exited with code {code}"),
            None => "process terminated by signal".to_owned(),
        })
    } else {
        None
    };

    let metadata = json!({
        "status": status,
        "stdout_bytes": output.stdout.original_len,
        "stderr_bytes": output.stderr.original_len,
        "stdout_truncated": output.stdout.truncated,
        "stderr_truncated": output.stderr.truncated,
        "timed_out": timed_out,
        "timeout_ms": TIMEOUT_MS,
        "external_terminal": external_terminal,
    });

    let result = json!({
        "call_id": call_id,
        "ok": success,
        "output": rendered,
        "content": [],
        "error": error_msg,
        "metadata": metadata
    });
    Ok(result.to_string())
}

fn should_use_ptyxis() -> bool {
    std::env::var(EXTERNAL_TERMINAL_ENV)
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case(PTYXIS_TERMINAL))
}

fn run_in_ptyxis(command: &str, cwd: &str, timeout: Duration) -> Result<(ShellOutput, bool)> {
    let capture_dir = tempfile::Builder::new()
        .prefix("agent-shell-ptyxis-")
        .tempdir()
        .with_context(|| "failed to create Ptyxis capture directory")?;
    let paths = PtyxisCapturePaths::new(capture_dir.path());
    fs::write(&paths.wrapper, ptyxis_wrapper_script())
        .with_context(|| format!("failed to write {}", paths.wrapper.display()))?;

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
printf '[agent] command:\n'
printf '%s\n\n' "$command_text"
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

fn spawn_ptyxis(command: &str, cwd: &str, paths: &PtyxisCapturePaths) -> Result<()> {
    let mut launcher = Command::new(PTYXIS_TERMINAL);
    if let Some(address) = std::env::var_os(EXTERNAL_TERMINAL_DBUS_ADDRESS_ENV) {
        launcher.env("DBUS_SESSION_BUS_ADDRESS", address);
    }
    let mut child = launcher
        .arg("--tab")
        .arg("--working-directory")
        .arg(cwd)
        .arg("--title")
        .arg(format!("agent shell · {}", command_summary(command)))
        .arg("--execute")
        .arg(ptyxis_execute_command(
            command,
            paths,
            std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(),
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| "failed to open Ptyxis terminal")?;
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
) -> Result<(ShellOutput, bool)> {
    let started = Instant::now();
    loop {
        if let Ok(status_text) = fs::read_to_string(&paths.status) {
            let code = status_text.trim().parse::<i32>().with_context(|| {
                format!("failed to parse Ptyxis command status: {status_text:?}")
            })?;
            return Ok((read_ptyxis_output(&paths, code)?, false));
        }
        if started.elapsed() >= timeout {
            kill_ptyxis_command(&paths.pid);
            return Ok((read_ptyxis_output(&paths, 124)?, true));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn read_ptyxis_output(paths: &PtyxisCapturePaths, code: i32) -> Result<ShellOutput> {
    Ok(ShellOutput {
        status: exit_status_from_code(code),
        stdout: read_bounded_file(&paths.stdout)?,
        stderr: read_bounded_file(&paths.stderr)?,
    })
}

fn read_bounded_file(path: &Path) -> Result<BoundedBuffer> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
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

fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> std::io::Result<(ShellOutput, bool)> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = spawn_bounded_reader(stdout);
    let stderr_reader = spawn_bounded_reader(stderr);
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

    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    Ok((
        ShellOutput {
            status,
            stdout,
            stderr,
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

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let tool: PluginToolObject = PluginTool_TO::from_value(ShellTool, TD_Opaque);
    registry.register_tool(tool)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("shell-tool"),
        description: RStr::from_str(
            "Shell tool plugin: opt-in RunsCommands tool, registers 'shell'",
        ),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn invoke(cwd: &std::path::Path, command: &str) -> Value {
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": {
                "command": command
            }
        });
        let result = invoke_impl(&call.to_string(), &cwd.display().to_string()).expect("invoke");
        serde_json::from_str(&result).expect("tool result")
    }

    #[test]
    fn shell_spec_allows_long_running_commands() {
        let spec: Value =
            serde_json::from_str(ShellTool.spec_json().as_str()).expect("tool spec json");

        assert_eq!(spec["timeout_ms"], TIMEOUT_MS);
        assert!(TIMEOUT_MS >= 600_000);
    }

    #[test]
    fn shell_runs_command_in_workspace() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("sample.txt"), "hello").expect("sample");

        let result = invoke(dir.path(), "pwd && cat sample.txt");

        assert_eq!(result["ok"], true);
        let output = result["output"].as_str().expect("output");
        assert!(output.contains(dir.path().to_str().unwrap()), "{output}");
        assert!(output.contains("hello"), "{output}");
        assert_eq!(result["metadata"]["timed_out"], false);
        assert_eq!(result["metadata"]["status"], 0);
    }

    #[test]
    fn shell_reports_nonzero_exit_as_failed_tool_result() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = invoke(dir.path(), "printf problem >&2; exit 7");

        assert_eq!(result["ok"], false);
        assert_eq!(result["output"], "problem");
        assert_eq!(result["error"], "process exited with code 7");
        assert_eq!(result["metadata"]["status"], 7);
    }

    #[test]
    fn shell_requires_command_arg() {
        let dir = tempfile::tempdir().expect("workspace");
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": {}
        });

        let error = invoke_impl(&call.to_string(), &dir.path().display().to_string())
            .expect_err("missing command must error");

        assert!(error.to_string().contains("requires string arg 'command'"));
    }

    #[test]
    fn timeout_kills_child_process_group() {
        let dir = tempfile::tempdir().expect("workspace");
        let child = spawn_shell("sleep 5", &dir.path().display().to_string()).expect("spawn shell");

        let (_output, timed_out) =
            wait_with_timeout(child, Duration::from_millis(100)).expect("wait with timeout");

        assert!(timed_out);
    }

    #[test]
    fn shell_truncates_large_output_without_blocking() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = invoke(dir.path(), "yes x | head -n 100000");

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["stdout_truncated"], true);
        assert!(result["metadata"]["stdout_bytes"].as_u64().unwrap() > OUTPUT_LIMIT_BYTES as u64);
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
        assert!(wrapper.contains("tee \"$stderr_path\""));
        assert!(wrapper.contains("printf '[agent] command:\\n'"));
        assert!(wrapper.contains("printf '%s\\n\\n' \"$command_text\""));
        assert!(wrapper.contains("printf '%s\\n' \"$status\""));
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
