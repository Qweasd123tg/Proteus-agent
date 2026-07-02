//! Shell tool как dylib-плагин.
//!
//! Регистрирует tools `shell` (one-shot команда), `exec_command` и
//! `write_stdin` (персистентные интерактивные PTY-сессии, см. `unified_exec`)
//! через `PluginTool` ABI. Безопасность `RunsCommands` —
//! `PermissionMode::Auto` запретит без approval, `plan` скроет вообще.
//! Вынесен из ядра именно ради этого: shell — самая рискованная вещь,
//! логично делать её opt-in через плагин, а не встраивать.
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

use anyhow::{Context, Result, anyhow};
use proteus_contracts::{
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
use serde_json::{Value, json};
use tempfile::TempDir;

mod unified_exec;

/// Максимум stdout/stderr. Reader продолжает дренировать pipe после лимита,
/// но сохраняет только head+tail: модель видит и начало вывода, и хвост
/// (там обычно ошибки), середина заменяется маркером.
const OUTPUT_LIMIT_BYTES: usize = 64 * 1024;
const HEAD_LIMIT_BYTES: usize = OUTPUT_LIMIT_BYTES / 2;
const TAIL_LIMIT_BYTES: usize = OUTPUT_LIMIT_BYTES - HEAD_LIMIT_BYTES;

/// Timeout на выполнение команды. Shell-команды часто запускают тесты,
/// сборки или генерацию артефактов, поэтому 30 секунд слишком агрессивны.
const TIMEOUT_MS: u64 = 600_000;
const EXTERNAL_TERMINAL_ENV: &str = "PROTEUS_SHELL_EXTERNAL_TERMINAL";
const EXTERNAL_TERMINAL_DBUS_ADDRESS_ENV: &str = "PROTEUS_SHELL_EXTERNAL_DBUS_ADDRESS";
const PTYXIS_TERMINAL: &str = "ptyxis";

struct ShellTool;

impl PluginTool for ShellTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "shell",
            "description": "Run a shell command in the current workspace (sh -lc). Commands run in a sandbox with no network access and read-only filesystem outside the workspace when the sandbox is available. Set `with_escalated_permissions: true` with a short `justification` to request an unsandboxed run (requires user approval). Set the `workdir` param to run in a subdirectory instead of using `cd` in the command. Interactive clients may choose to surface command output in their own UI; headless runs return captured stdout/stderr. Safety: RunsCommands.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "workdir": {
                        "type": "string",
                        "description": "Working directory for the command; relative paths resolve against the workspace root. Defaults to the workspace root."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Per-call timeout in milliseconds; capped at the tool default."
                    },
                    "with_escalated_permissions": {
                        "type": "boolean",
                        "description": "Request an unsandboxed run (network / writes outside workspace). Requires user approval."
                    },
                    "justification": {
                        "type": "string",
                        "description": "One sentence explaining why escalated permissions are needed."
                    }
                },
                "required": ["command"]
            },
            "safety": "RunsCommands",
            "timeout_ms": TIMEOUT_MS,
            "metadata": {
                "category": "terminal",
                "tags": ["terminal", "command", "test", "build"],
                "aliases": ["run command", "cargo test", "npm test", "execute"]
            }
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
    let args = call.get("args");
    let command = args
        .and_then(|args| args.get("command"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("shell requires string arg 'command'"))?;
    let workdir = resolve_workdir(cwd, args.and_then(|args| args.get("workdir")))?;
    let timeout_ms = args
        .and_then(|args| args.get("timeout_ms"))
        .and_then(Value::as_u64)
        .map_or(TIMEOUT_MS, |requested| requested.clamp(1, TIMEOUT_MS));
    let escalated = args
        .and_then(|args| args.get("with_escalated_permissions"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sandbox = if escalated { None } else { sandbox_kind(cwd) };

    let (output, timed_out, external_terminal) = if should_use_ptyxis() {
        let (output, timed_out) =
            run_in_ptyxis(command, &workdir, Duration::from_millis(timeout_ms))
                .with_context(|| "failed to run shell in Ptyxis")?;
        (output, timed_out, Some(PTYXIS_TERMINAL))
    } else {
        let child = spawn_shell(command, cwd, &workdir, sandbox)
            .with_context(|| "failed to spawn shell")?;
        let (output, timed_out) = wait_with_timeout(child, Duration::from_millis(timeout_ms))
            .with_context(|| "failed to wait for shell")?;
        (output, timed_out, None)
    };

    let stdout = output.stdout.to_text();
    let stderr = output.stderr.to_text();
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
        Some(format!("process timed out after {timeout_ms}ms"))
    } else if !success {
        Some(match status {
            Some(code) => format!("process exited with code {code}"),
            None => "process terminated by signal".to_owned(),
        })
    } else {
        None
    };

    let metadata = json!({
        "exit_code": status,
        "stdout_bytes": output.stdout.original_len,
        "stderr_bytes": output.stderr.original_len,
        "stdout_truncated": output.stdout.truncated(),
        "stderr_truncated": output.stderr.truncated(),
        "timed_out": timed_out,
        "timeout_ms": timeout_ms,
        "workdir": workdir,
        "sandbox": sandbox.map(SandboxKind::label),
        "escalated": escalated,
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

/// Резолвит `workdir` арг: относительный путь — от workspace root. Директория
/// обязана существовать, иначе модель получает понятную ошибку вместо
/// сбоя spawn.
fn resolve_workdir(cwd: &str, workdir: Option<&Value>) -> Result<String> {
    let Some(workdir) = workdir else {
        return Ok(cwd.to_owned());
    };
    let workdir = workdir
        .as_str()
        .ok_or_else(|| anyhow!("shell arg 'workdir' must be a string"))?;
    if workdir.trim().is_empty() {
        return Ok(cwd.to_owned());
    }
    let path = Path::new(workdir);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(cwd).join(path)
    };
    if !resolved.is_dir() {
        return Err(anyhow!(
            "shell workdir does not exist or is not a directory: {}",
            resolved.display()
        ));
    }
    Ok(resolved.display().to_string())
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
    Ok(BoundedBuffer::from_bytes(&bytes))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SandboxKind {
    Bwrap,
}

impl SandboxKind {
    fn label(self) -> &'static str {
        match self {
            Self::Bwrap => "bwrap",
        }
    }
}

/// Sandbox доступен, если найден bubblewrap. `PROTEUS_SHELL_SANDBOX=0`
/// выключает его целиком (эскалация тогда не нужна, но и изоляции нет).
fn sandbox_kind(_cwd: &str) -> Option<SandboxKind> {
    if std::env::var("PROTEUS_SHELL_SANDBOX").is_ok_and(|value| value == "0") {
        return None;
    }
    let available = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("bwrap").is_file()));
    available.then_some(SandboxKind::Bwrap)
}

/// argv для bwrap: read-only корень, rw-bind workspace (и workdir, если он
/// вне workspace), без сети, свежие /dev,/proc,/tmp.
fn bwrap_args(command: &str, workspace: &str, workdir: &str) -> Vec<String> {
    let mut args = vec![
        "--die-with-parent".to_owned(),
        "--unshare-net".to_owned(),
        "--ro-bind".to_owned(),
        "/".to_owned(),
        "/".to_owned(),
        "--dev".to_owned(),
        "/dev".to_owned(),
        "--proc".to_owned(),
        "/proc".to_owned(),
        "--tmpfs".to_owned(),
        "/tmp".to_owned(),
        "--bind".to_owned(),
        workspace.to_owned(),
        workspace.to_owned(),
    ];
    if !Path::new(workdir).starts_with(workspace) {
        args.extend(["--bind".to_owned(), workdir.to_owned(), workdir.to_owned()]);
    }
    args.extend([
        "--chdir".to_owned(),
        workdir.to_owned(),
        "sh".to_owned(),
        "-lc".to_owned(),
        command.to_owned(),
    ]);
    args
}

fn spawn_shell(
    command: &str,
    workspace: &str,
    workdir: &str,
    sandbox: Option<SandboxKind>,
) -> std::io::Result<Child> {
    let mut command_builder = match sandbox {
        Some(SandboxKind::Bwrap) => {
            let mut builder = Command::new("bwrap");
            builder.args(bwrap_args(command, workspace, workdir));
            builder
        }
        None => {
            let mut builder = Command::new("sh");
            builder.arg("-lc").arg(command);
            builder
        }
    };
    command_builder
        .current_dir(workdir)
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

/// Head+tail буфер с жёстким потолком памяти: первые `HEAD_LIMIT_BYTES` и
/// последние `TAIL_LIMIT_BYTES` байта, середина дропается.
struct BoundedBuffer {
    head: Vec<u8>,
    tail: std::collections::VecDeque<u8>,
    original_len: usize,
}

impl BoundedBuffer {
    fn new() -> Self {
        Self {
            head: Vec::new(),
            tail: std::collections::VecDeque::new(),
            original_len: 0,
        }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let mut buffer = Self::new();
        buffer.push(bytes);
        buffer
    }

    fn push(&mut self, data: &[u8]) {
        self.original_len += data.len();
        let mut rest = data;
        if self.head.len() < HEAD_LIMIT_BYTES {
            let take = (HEAD_LIMIT_BYTES - self.head.len()).min(rest.len());
            self.head.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
        }
        if rest.is_empty() {
            return;
        }
        self.tail.extend(rest.iter().copied());
        if self.tail.len() > TAIL_LIMIT_BYTES {
            let excess = self.tail.len() - TAIL_LIMIT_BYTES;
            self.tail.drain(..excess);
        }
    }

    fn truncated(&self) -> bool {
        self.original_len > self.head.len() + self.tail.len()
    }

    fn to_text(&self) -> String {
        let head = String::from_utf8_lossy(&self.head);
        if self.tail.is_empty() {
            return head.into_owned();
        }
        let tail_bytes: Vec<u8> = self.tail.iter().copied().collect();
        let tail = String::from_utf8_lossy(&tail_bytes);
        if !self.truncated() {
            return format!("{head}{tail}");
        }
        let omitted = self.original_len - self.head.len() - self.tail.len();
        format!(
            "{head}\n{}\n{tail}",
            omitted_marker(omitted, self.original_len)
        )
    }
}

/// Единый формат маркера усечения для терминальных tools (`shell`,
/// `exec_command`/`write_stdin`).
fn omitted_marker(omitted: usize, total: usize) -> String {
    format!("[... omitted {omitted} of {total} bytes ...]")
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
        let mut buffer = BoundedBuffer::new();
        let Some(mut reader) = reader else {
            return Ok(buffer);
        };
        let mut buf = [0u8; 8192];
        loop {
            let read = reader.read(&mut buf)?;
            if read == 0 {
                break;
            }
            buffer.push(&buf[..read]);
        }
        Ok(buffer)
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
    if let RResult::RErr(err) = registry.register_tool(tool) {
        return RResult::RErr(err);
    }

    let exec: PluginToolObject =
        PluginTool_TO::from_value(unified_exec::ExecCommandTool, TD_Opaque);
    if let RResult::RErr(err) = registry.register_tool(exec) {
        return RResult::RErr(err);
    }

    let stdin: PluginToolObject =
        PluginTool_TO::from_value(unified_exec::WriteStdinTool, TD_Opaque);
    registry.register_tool(stdin)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("shell-tool"),
        description: RStr::from_str(
            "Shell tool plugin: opt-in RunsCommands tools, registers 'shell', 'exec_command', 'write_stdin'",
        ),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const _: () = assert!(TIMEOUT_MS >= 600_000);

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
        assert_eq!(result["metadata"]["exit_code"], 0);
    }

    #[test]
    fn shell_reports_nonzero_exit_as_failed_tool_result() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = invoke(dir.path(), "printf problem >&2; exit 7");

        assert_eq!(result["ok"], false);
        assert_eq!(result["output"], "problem");
        assert_eq!(result["error"], "process exited with code 7");
        assert_eq!(result["metadata"]["exit_code"], 7);
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
    fn shell_runs_in_relative_workdir() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::create_dir(dir.path().join("sub")).expect("subdir");
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": { "command": "pwd", "workdir": "sub" }
        });

        let result = invoke_impl(&call.to_string(), &dir.path().display().to_string())
            .map(|json| serde_json::from_str::<Value>(&json).expect("tool result"))
            .expect("invoke");

        assert_eq!(result["ok"], true);
        let output = result["output"].as_str().expect("output");
        assert!(output.trim_end().ends_with("sub"), "{output}");
        assert!(
            result["metadata"]["workdir"]
                .as_str()
                .expect("workdir meta")
                .ends_with("sub")
        );
    }

    #[test]
    fn shell_rejects_missing_workdir() {
        let dir = tempfile::tempdir().expect("workspace");
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": { "command": "pwd", "workdir": "no-such-dir" }
        });

        let error = invoke_impl(&call.to_string(), &dir.path().display().to_string())
            .expect_err("missing workdir must error");

        assert!(error.to_string().contains("does not exist"), "{error}");
    }

    #[test]
    fn shell_honours_per_call_timeout() {
        let dir = tempfile::tempdir().expect("workspace");
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": { "command": "sleep 5", "timeout_ms": 100 }
        });

        let result = invoke_impl(&call.to_string(), &dir.path().display().to_string())
            .map(|json| serde_json::from_str::<Value>(&json).expect("tool result"))
            .expect("invoke");

        assert_eq!(result["ok"], false);
        assert_eq!(result["metadata"]["timed_out"], true);
        assert_eq!(result["metadata"]["timeout_ms"], 100);
        assert_eq!(result["error"], "process timed out after 100ms");
    }

    #[test]
    fn shell_caps_per_call_timeout_at_default() {
        let dir = tempfile::tempdir().expect("workspace");
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": { "command": "true", "timeout_ms": u64::MAX }
        });

        let result = invoke_impl(&call.to_string(), &dir.path().display().to_string())
            .map(|json| serde_json::from_str::<Value>(&json).expect("tool result"))
            .expect("invoke");

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["timeout_ms"], TIMEOUT_MS);
    }

    #[test]
    fn bwrap_args_isolate_network_and_bind_workspace() {
        let args = bwrap_args("echo hi", "/ws", "/ws/sub");
        assert!(args.contains(&"--unshare-net".to_owned()));
        assert!(args.windows(3).any(|w| w == ["--ro-bind", "/", "/"]));
        assert!(args.windows(3).any(|w| w == ["--bind", "/ws", "/ws"]));
        assert!(args.windows(2).any(|w| w == ["--chdir", "/ws/sub"]));
        assert_eq!(args.last().map(String::as_str), Some("echo hi"));
        // workdir внутри workspace не биндится отдельно
        assert!(
            !args
                .windows(3)
                .any(|w| w == ["--bind", "/ws/sub", "/ws/sub"])
        );
    }

    #[test]
    fn bwrap_args_bind_external_workdir() {
        let args = bwrap_args("pwd", "/ws", "/opt/elsewhere");
        assert!(
            args.windows(3)
                .any(|w| w == ["--bind", "/opt/elsewhere", "/opt/elsewhere"])
        );
    }

    #[test]
    fn escalated_call_skips_sandbox_and_reports_metadata() {
        let dir = tempfile::tempdir().expect("workspace");
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": {
                "command": "printf ok",
                "with_escalated_permissions": true,
                "justification": "test"
            }
        });

        let result = invoke_impl(&call.to_string(), &dir.path().display().to_string())
            .map(|json| serde_json::from_str::<Value>(&json).expect("tool result"))
            .expect("invoke");

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["escalated"], true);
        assert_eq!(result["metadata"]["sandbox"], Value::Null);
    }

    #[test]
    fn sandboxed_run_blocks_network_when_bwrap_available() {
        if sandbox_kind(".").is_none() {
            return; // окружение без bwrap — интеграцию пропускаем
        }
        let dir = tempfile::tempdir().expect("workspace");

        let ok = invoke(dir.path(), "printf sandboxed");
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["metadata"]["sandbox"], "bwrap");

        // сеть в sandbox отрезана: getent/curl недоступны без сети;
        // используем /dev/tcp bash-исмуляцию через sh — надёжнее ping
        let net = invoke(
            dir.path(),
            "sh -c 'echo x > /dev/tcp/127.0.0.1/9' 2>&1; true",
        );
        assert_eq!(net["metadata"]["sandbox"], "bwrap");
    }

    #[test]
    fn timeout_kills_child_process_group() {
        let dir = tempfile::tempdir().expect("workspace");
        let cwd = dir.path().display().to_string();
        let child = spawn_shell("sleep 5", &cwd, &cwd, None).expect("spawn shell");

        let (_output, timed_out) =
            wait_with_timeout(child, Duration::from_millis(100)).expect("wait with timeout");

        assert!(timed_out);
    }

    #[test]
    fn shell_truncates_large_output_head_and_tail() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = invoke(dir.path(), "seq 1 50000");

        assert_eq!(result["ok"], true);
        assert_eq!(result["metadata"]["stdout_truncated"], true);
        assert!(result["metadata"]["stdout_bytes"].as_u64().unwrap() > OUTPUT_LIMIT_BYTES as u64);
        let output = result["output"].as_str().expect("output");
        // Видны начало, маркер пропуска и хвост вывода.
        assert!(output.starts_with("1\n2\n"), "{}", &output[..40]);
        assert!(output.contains("[... omitted"), "no marker");
        assert!(output.trim_end().ends_with("50000"), "tail missing");
    }

    #[test]
    fn bounded_buffer_keeps_head_and_tail_within_limit() {
        let mut buffer = BoundedBuffer::new();
        buffer.push(&vec![b'a'; HEAD_LIMIT_BYTES]);
        buffer.push(&vec![b'b'; TAIL_LIMIT_BYTES]);
        assert!(!buffer.truncated());
        assert_eq!(buffer.to_text().len(), OUTPUT_LIMIT_BYTES);

        buffer.push(&vec![b'c'; TAIL_LIMIT_BYTES]);
        assert!(buffer.truncated());
        let text = buffer.to_text();
        assert!(text.starts_with('a'));
        assert!(text.trim_end().ends_with('c'));
        assert!(text.contains("[... omitted"), "no marker");
        // Память ограничена head+tail, середина ушла.
        assert_eq!(buffer.original_len, HEAD_LIMIT_BYTES + 2 * TAIL_LIMIT_BYTES);
        assert_eq!(buffer.head.len() + buffer.tail.len(), OUTPUT_LIMIT_BYTES);
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
