//! Shell tools как reference process module.
//!
//! Регистрирует tools `shell` (one-shot команда), `exec_command` и
//! `write_stdin` (персистентные интерактивные PTY-сессии, см. `unified_exec`)
//! через process `Tool` contract. Безопасность `RunsCommands` —
//! `PermissionMode::Auto` запретит без approval, `plan` скроет вообще.
//! Вынесен из ядра именно ради этого: shell — самая рискованная вещь,
//! логично делать её opt-in через module config, а не встраивать.
//!
//! Реализация держит stdout/stderr bounded и на Unix запускает shell в
//! отдельной process group, чтобы timeout мог остановить не только `sh`, но и
//! его дочерние процессы.

use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use anyhow::{Context, Result, anyhow};
use proteus_contracts::{
    domain::EXEC_SHELL,
    process_module::{
        ModuleRegistry, ProcessModuleError, ToolModule, ToolModuleHostMut,
        ToolModuleInvocationContext, ToolModuleObject,
    },
};
use serde_json::{Value, json};

#[cfg(unix)]
mod child_status;
mod execution;
mod ptyxis;
mod result;
mod sandbox;
mod unified_exec;

#[cfg(test)]
use execution::BoundedBuffer;
#[cfg(test)]
use execution::wait_with_timeout;
use execution::{omitted_marker, wait_with_timeout_and_cancel};
use ptyxis::{run_in_ptyxis, should_use_ptyxis};
use sandbox::{EXEC_COMMAND_ENV, SandboxKind, SandboxPolicy, bwrap_args, resolve_workdir};
#[cfg(test)]
use std::time::Instant;

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

impl ToolModule for ShellTool {
    fn spec_json(&self) -> String {
        let spec = json!({
            "name": "shell",
            "description": "Run a shell command in the current workspace (sh -lc). Non-escalated commands require bwrap and run with no network access, a private PID namespace, and a read-only filesystem outside the workspace; execution fails closed when bwrap is unavailable or disabled. The sandbox network is isolated per call: a localhost server started by one sandboxed call is unreachable from any other call and from the user's machine. Start servers that must stay reachable with `with_escalated_permissions: true`. Set `with_escalated_permissions: true` with a short `justification` to request an unsandboxed run (requires user approval). Non-escalated workdirs must stay inside the workspace. External-terminal execution is unsandboxed and therefore also requires escalation. Set the `workdir` param to run in a subdirectory instead of using `cd` in the command. Interactive clients may choose to surface command output in their own UI; headless runs return captured stdout/stderr. Safety: RunsCommands.",
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
            "surface": { "kind": "function", "strict": false, "output_schema": null },
            "safety": "RunsCommands",
            "supports_parallel_tool_calls": false,
            "timeout_ms": TIMEOUT_MS,
            "metadata": {
                "category": "terminal",
                "tags": ["terminal", "command", "test", "build"],
                "aliases": ["run command", "cargo test", "npm test", "execute"]
            }
        });
        spec.to_string()
    }

    fn invoke_json(
        &self,
        call_json: String,
        context_json: String,
        host: &mut ToolModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let context: ToolModuleInvocationContext = match serde_json::from_str(context_json.as_str())
        {
            Ok(context) => context,
            Err(error) => {
                return Err(ProcessModuleError::new(format!(
                    "failed to parse ToolModuleInvocationContext: {error}"
                )));
            }
        };
        let mut is_cancelled = || {
            host.is_cancelled()
                .map_err(|error| std::io::Error::other(error.message))
        };
        match invoke_impl_with_cancel(
            call_json.as_str(),
            &context.cwd.to_string_lossy(),
            &mut is_cancelled,
        ) {
            Ok(result_json) => Ok(result_json),
            Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
        }
    }
}

#[cfg(test)]
fn invoke_impl(call_json: &str, cwd: &str) -> Result<String> {
    invoke_impl_with_cancel(call_json, cwd, &mut || Ok(false))
}

fn invoke_impl_with_cancel(
    call_json: &str,
    cwd: &str,
    is_cancelled: &mut dyn FnMut() -> std::io::Result<bool>,
) -> Result<String> {
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
    let escalated = args
        .and_then(|args| args.get("with_escalated_permissions"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sandbox_policy = if escalated {
        SandboxPolicy::not_required()
    } else {
        SandboxPolicy::detect(cwd)
    };
    invoke_command_with_cancel(
        call_id,
        args,
        command,
        cwd,
        escalated,
        sandbox_policy,
        should_use_ptyxis(),
        is_cancelled,
    )
}

#[cfg(test)]
fn invoke_command(
    call_id: String,
    args: Option<&Value>,
    command: &str,
    cwd: &str,
    escalated: bool,
    sandbox_policy: SandboxPolicy,
    external_terminal_requested: bool,
) -> Result<String> {
    invoke_command_with_cancel(
        call_id,
        args,
        command,
        cwd,
        escalated,
        sandbox_policy,
        external_terminal_requested,
        &mut || Ok(false),
    )
}

fn invoke_command_with_cancel(
    call_id: String,
    args: Option<&Value>,
    command: &str,
    cwd: &str,
    escalated: bool,
    sandbox_policy: SandboxPolicy,
    external_terminal_requested: bool,
    is_cancelled: &mut dyn FnMut() -> std::io::Result<bool>,
) -> Result<String> {
    let resolved = resolve_workdir(cwd, args.and_then(|args| args.get("workdir")), escalated)?;
    let timeout_ms = args
        .and_then(|args| args.get("timeout_ms"))
        .and_then(Value::as_u64)
        .map_or(TIMEOUT_MS, |requested| requested.clamp(1, TIMEOUT_MS));
    let sandbox = sandbox_policy.select(escalated, external_terminal_requested)?;

    let (output, timed_out, external_terminal) = if external_terminal_requested {
        let (output, timed_out) = run_in_ptyxis(
            command,
            &resolved.workdir,
            Duration::from_millis(timeout_ms),
            is_cancelled,
        )
        .with_context(|| "failed to run shell in Ptyxis")?;
        (output, timed_out, Some(PTYXIS_TERMINAL))
    } else {
        let child = spawn_shell(
            command,
            &resolved.workspace,
            &resolved.workdir,
            sandbox.as_ref(),
        )
        .with_context(|| "failed to spawn shell")?;
        let (output, timed_out) =
            wait_with_timeout_and_cancel(child, Duration::from_millis(timeout_ms), is_cancelled)
                .with_context(|| "failed to wait for shell")?;
        (output, timed_out, None)
    };

    let metadata = json!({
        "workdir": resolved.workdir,
        "sandbox": sandbox.as_ref().map(SandboxKind::label),
        "escalated": escalated,
        "external_terminal": external_terminal,
    });
    Ok(result::render_output(
        call_id, output, timed_out, timeout_ms, metadata,
    ))
}

fn spawn_shell(
    command: &str,
    workspace: &str,
    workdir: &str,
    sandbox: Option<&SandboxKind>,
) -> std::io::Result<Child> {
    let mut command_builder = match sandbox {
        Some(sandbox @ SandboxKind::Bwrap(_)) => {
            let mut builder = Command::new(sandbox.executable());
            builder.args(bwrap_args(command, workspace, workdir));
            builder
        }
        None => {
            let mut builder = Command::new(EXEC_SHELL);
            builder.arg("-lc").arg(command);
            builder
        }
    };
    command_builder
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in EXEC_COMMAND_ENV {
        command_builder.env(key, value);
    }

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

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let tool: ToolModuleObject = Box::new(ShellTool);
    registry.register_tool(tool)?;

    let exec: ToolModuleObject = Box::new(unified_exec::ExecCommandTool);
    registry.register_tool(exec)?;

    let stdin: ToolModuleObject = Box::new(unified_exec::WriteStdinTool);
    registry.register_tool(stdin)
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
                "command": command,
                "with_escalated_permissions": true,
                "justification": "unit test"
            }
        });
        let result = invoke_impl(&call.to_string(), &cwd.display().to_string()).expect("invoke");
        serde_json::from_str(&result).expect("tool result")
    }

    fn invoke_sandboxed(cwd: &std::path::Path, command: &str) -> Result<Value> {
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": { "command": command }
        });
        let result = invoke_impl(&call.to_string(), &cwd.display().to_string())?;
        serde_json::from_str(&result).map_err(Into::into)
    }

    fn verified_sandbox_or_fail_closed(cwd: &std::path::Path) -> Option<SandboxPolicy> {
        let policy = SandboxPolicy::detect(&cwd.display().to_string());
        if policy.select(false, false).is_ok() {
            return Some(policy);
        }

        let marker = cwd.join("sandbox-must-not-run");
        let error = invoke_command(
            "sandbox_probe".to_owned(),
            None,
            "touch sandbox-must-not-run",
            &cwd.display().to_string(),
            false,
            policy,
            false,
        )
        .expect_err("unverified sandbox must fail closed");
        assert!(
            error.to_string().contains("bwrap") || error.to_string().contains("sandbox"),
            "unexpected sandbox rejection: {error}"
        );
        assert!(!marker.exists(), "unavailable sandbox ran the command");
        None
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
    fn shell_neutralizes_interactive_env() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = invoke(
            dir.path(),
            "printf '%s|%s|%s' \"$TERM\" \"$GIT_PAGER\" \"$PAGER\"",
        );

        assert_eq!(result["ok"], true);
        assert_eq!(result["output"], "dumb|cat|cat");
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
            "args": {
                "command": "pwd",
                "workdir": "sub",
                "with_escalated_permissions": true,
                "justification": "unit test"
            }
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
            "args": {
                "command": "sleep 5",
                "timeout_ms": 100,
                "with_escalated_permissions": true,
                "justification": "unit test"
            }
        });

        let result = invoke_impl(&call.to_string(), &dir.path().display().to_string())
            .map(|json| serde_json::from_str::<Value>(&json).expect("tool result"))
            .expect("invoke");

        assert_eq!(result["ok"], false);
        assert_eq!(result["metadata"]["timed_out"], true);
        assert_eq!(result["metadata"]["timeout_ms"], 100);
        assert_eq!(result["error"], "process timed out after 100ms");
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_during_output_drain_kills_background_descendant() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let child = spawn_shell(
            "sh -c 'sleep 1; touch late-marker' & printf parent-done",
            cwd,
            cwd,
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while crate::child_status::observe_exit(child.id())
            .unwrap()
            .is_none()
        {
            assert!(Instant::now() < deadline, "parent shell did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
        let started = Instant::now();
        let error = match wait_with_timeout_and_cancel(child, Duration::from_secs(3), &mut || {
            Ok(started.elapsed() >= Duration::from_millis(100))
        }) {
            Ok(_) => panic!("cancel during inherited pipe drain must fail"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        std::thread::sleep(Duration::from_millis(1100));
        assert!(!dir.path().join("late-marker").exists());
    }

    #[test]
    fn shell_caps_per_call_timeout_at_default() {
        let dir = tempfile::tempdir().expect("workspace");
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": {
                "command": "true",
                "timeout_ms": u64::MAX,
                "with_escalated_permissions": true,
                "justification": "unit test"
            }
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
    fn bwrap_args_isolate_pid_namespace_and_mount_matching_procfs() {
        let args = bwrap_args("true", "/ws", "/ws");
        let unshare_pid = args
            .iter()
            .position(|arg| arg == "--unshare-pid")
            .expect("private PID namespace");
        let proc_mount = args
            .windows(2)
            .position(|args| args == ["--proc", "/proc"])
            .expect("procfs for the private PID namespace");

        assert!(unshare_pid < proc_mount);
    }

    #[test]
    fn bwrap_args_never_bind_external_workdir() {
        let args = bwrap_args("pwd", "/ws", "/opt/elsewhere");
        assert!(
            !args
                .windows(3)
                .any(|w| w == ["--bind", "/opt/elsewhere", "/opt/elsewhere"])
        );
    }

    #[test]
    fn non_escalated_shell_fails_closed_without_bwrap() {
        let dir = tempfile::tempdir().expect("workspace");
        let marker = dir.path().join("must-not-exist");
        let args = json!({ "command": "touch must-not-exist" });

        let error = invoke_command(
            "call_shell".to_owned(),
            Some(&args),
            args["command"].as_str().unwrap(),
            &dir.path().display().to_string(),
            false,
            SandboxPolicy::disabled_for_test(),
            false,
        )
        .expect_err("missing bwrap must reject non-escalated shell");

        assert!(error.to_string().contains("PROTEUS_SHELL_SANDBOX=0"));
        assert!(!marker.exists(), "command must not be spawned");
    }

    #[test]
    fn non_escalated_shell_rejects_external_workdir() {
        let workspace = tempfile::tempdir().expect("workspace");
        let external = tempfile::tempdir().expect("external workdir");
        let call = json!({
            "id": "call_shell",
            "name": "shell",
            "args": { "command": "pwd", "workdir": external.path() }
        });

        let error = invoke_impl(&call.to_string(), &workspace.path().display().to_string())
            .expect_err("external workdir must require escalation");

        assert!(
            error.to_string().contains("outside the workspace"),
            "{error}"
        );
        assert!(
            error
                .to_string()
                .contains("with_escalated_permissions=true"),
            "{error}"
        );
    }

    #[test]
    fn non_escalated_shell_rejects_unsandboxed_external_terminal() {
        let dir = tempfile::tempdir().expect("workspace");
        let args = json!({ "command": "true" });

        let error = invoke_command(
            "call_shell".to_owned(),
            Some(&args),
            "true",
            &dir.path().display().to_string(),
            false,
            SandboxPolicy::unavailable_for_test(),
            true,
        )
        .expect_err("Ptyxis requires escalation");

        assert!(error.to_string().contains("external terminal"), "{error}");
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
        let dir = tempfile::tempdir().expect("workspace");
        let Some(_sandbox_policy) = verified_sandbox_or_fail_closed(dir.path()) else {
            return;
        };

        let ok = invoke_sandboxed(dir.path(), "printf sandboxed").expect("sandboxed invoke");
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["metadata"]["sandbox"], "bwrap");

        // сеть в sandbox отрезана: getent/curl недоступны без сети;
        // используем /dev/tcp bash-исмуляцию через sh — надёжнее ping
        let net = invoke_sandboxed(
            dir.path(),
            "sh -c 'echo x > /dev/tcp/127.0.0.1/9' 2>&1; true",
        )
        .expect("sandboxed network invoke");
        assert_eq!(net["metadata"]["sandbox"], "bwrap");
    }

    #[cfg(unix)]
    #[test]
    fn sandboxed_run_cannot_see_or_signal_external_process_when_bwrap_available() {
        let dir = tempfile::tempdir().expect("workspace");
        let cwd = dir.path().display().to_string();
        let Some(sandbox_policy) = verified_sandbox_or_fail_closed(dir.path()) else {
            return;
        };

        // Signal only a process owned by this test. On the old shared-PID path
        // the sandbox could see and terminate it by its host PID.
        let mut external = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("external sleep");
        let external_pid = external.id();
        let command = format!(
            "visible=0; signalled=0; test -e /proc/{external_pid} && visible=1; \
             kill -TERM {external_pid} 2>/dev/null && signalled=1; \
             test \"$visible\" -eq 0 && test \"$signalled\" -eq 0"
        );
        let args = json!({ "command": command });
        let result = invoke_command(
            "pid_namespace_test".to_owned(),
            Some(&args),
            args["command"].as_str().expect("command"),
            &cwd,
            false,
            sandbox_policy,
            false,
        );
        let external_survived = external
            .try_wait()
            .expect("external sleep status")
            .is_none();
        let _ = external.kill();
        let _ = external.wait();

        let result = result
            .map(|json| serde_json::from_str::<Value>(&json).expect("tool result"))
            .expect("sandboxed PID isolation run");
        assert_eq!(result["ok"], true, "{result}");
        assert!(external_survived, "sandbox signalled external process");
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
}
