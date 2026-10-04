//! External Ptyxis wrapper, completion publication and output capture.
use crate::execution::{BoundedBuffer, ShellOutput};
use crate::{EXTERNAL_TERMINAL_DBUS_ADDRESS_ENV, EXTERNAL_TERMINAL_ENV, PTYXIS_TERMINAL};
use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;

pub(super) fn should_use_ptyxis() -> bool {
    std::env::var(EXTERNAL_TERMINAL_ENV)
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case(PTYXIS_TERMINAL))
}

pub(super) fn run_in_ptyxis(
    command: &str,
    cwd: &str,
    timeout: Duration,
    is_cancelled: &mut dyn FnMut() -> std::io::Result<bool>,
) -> Result<(ShellOutput, bool)> {
    let capture_dir = tempfile::Builder::new()
        .prefix("agent-shell-ptyxis-")
        .tempdir()
        .with_context(|| "failed to create Ptyxis capture directory")?;
    let paths = PtyxisCapturePaths::new(capture_dir.path());
    fs::write(&paths.wrapper, ptyxis_wrapper_script())
        .with_context(|| format!("failed to write {}", paths.wrapper.display()))?;

    spawn_ptyxis(command, cwd, &paths)?;
    wait_for_ptyxis_result(capture_dir, paths, timeout, is_cancelled)
}

struct PtyxisCapturePaths {
    wrapper: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
    status: PathBuf,
    cancel: PathBuf,
}

impl PtyxisCapturePaths {
    fn new(dir: &Path) -> Self {
        Self {
            wrapper: dir.join("run.sh"),
            stdout: dir.join("stdout.log"),
            stderr: dir.join("stderr.log"),
            status: dir.join("status"),
            cancel: dir.join("cancel"),
        }
    }
}

fn ptyxis_wrapper_script() -> &'static str {
    r#"#!/usr/bin/env bash
set +e
capture_mode=0
if [ "$1" = --capture ]; then capture_mode=1; shift; fi
command_text="$1"
stdout_path="$2"
stderr_path="$3"
status_path="$4"
cancel_path="$5"
publish_status() {
    printf '%s %s %s\n' "$1" "$2" "$3" > "$status_path.tmp" &&
        mv -f -- "$status_path.tmp" "$status_path"
}
if [ "$capture_mode" = 0 ]; then
    finish() {
        trap - HUP INT TERM
        printf cancel > "$cancel_path"
        if [ -n "${supervisor_pid:-}" ]; then wait "$supervisor_pid"; fi
        exit 130
    }
    trap 'finish' HUP INT TERM
    printf '[agent] command:\n'
    printf '%s\n\n' "$command_text"
    if [ -e "$cancel_path" ]; then publish_status 130 0 0; exit 130; fi
    setsid bash "$0" --capture "$@" &
    supervisor_pid=$!
    wait "$supervisor_pid"
    supervisor_status=$?
    trap - HUP INT TERM
    if [ ! -e "$status_path" ]; then
        publish_status "$supervisor_status" "$supervisor_status" "$supervisor_status"
    fi
    printf '\n[agent] capture completed\n'
    exec bash --noprofile --norc -i
fi
# This shell remains the group leader until command, drains and watcher exit.
# The watcher signals its own group, whose identity its membership reserves.
trap 'kill -KILL 0' HUP INT TERM
(
    while :; do
        if [ -e "$cancel_path" ]; then kill -KILL 0; fi
        if [ -e "$status_path.drained" ]; then exit 0; fi
        sleep 0.01
    done
) &
watcher_pid=$!
exec {stdout_fd}> >(tee "$stdout_path")
stdout_pid=$!
exec {stderr_fd}> >(exec {stdout_fd}>&-; tee "$stderr_path" >&2)
stderr_pid=$!
(exec {stdout_fd}>&- {stderr_fd}>&-; exec sh -lc "$command_text") >&$stdout_fd 2>&$stderr_fd &
command_pid=$!
exec {stdout_fd}>&-
exec {stderr_fd}>&-
wait "$command_pid"
status=$?
wait "$stdout_pid"
stdout_status=$?
wait "$stderr_pid"
stderr_status=$?
printf done > "$status_path.drained"
wait "$watcher_pid"
trap - HUP INT TERM
publish_status "$status" "$stdout_status" "$stderr_status"
exit "$status"
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
        paths.cancel.display().to_string(),
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
    capture_dir: TempDir,
    paths: PtyxisCapturePaths,
    timeout: Duration,
    is_cancelled: &mut dyn FnMut() -> std::io::Result<bool>,
) -> Result<(ShellOutput, bool)> {
    let started = Instant::now();
    loop {
        match is_cancelled() {
            Ok(false) => {}
            result => {
                if !stop_ptyxis_command(&paths) {
                    // Keep the request reachable if terminal startup/scheduling
                    // delayed acknowledgement beyond the bounded cleanup wait.
                    let _ = capture_dir.keep();
                }
                result.context("failed to check shell cancellation")?;
                anyhow::bail!("shell invocation canceled");
            }
        }
        if let Ok(status_text) = fs::read_to_string(&paths.status) {
            let codes = status_text
                .split_whitespace()
                .map(str::parse::<i32>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .with_context(|| {
                    format!("failed to parse Ptyxis command status: {status_text:?}")
                })?;
            anyhow::ensure!(
                codes.len() == 3,
                "invalid Ptyxis completion marker: {status_text:?}"
            );
            let code = codes[0];
            anyhow::ensure!(
                codes[1] == 0 && codes[2] == 0,
                "Ptyxis output capture failed (stdout drain: {}, stderr drain: {}, command exit: {code})",
                codes[1],
                codes[2]
            );
            return Ok((read_ptyxis_output(&paths, code)?, false));
        }
        if started.elapsed() >= timeout {
            let stopped = stop_ptyxis_command(&paths);
            let output = read_ptyxis_output(&paths, 124);
            if !stopped {
                let _ = capture_dir.keep();
            }
            return Ok((output?, true));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn stop_ptyxis_command(paths: &PtyxisCapturePaths) -> bool {
    // The live supervisor's watcher owns group signalling. The host never
    // retains a numeric PID that could outlive the external command.
    let _ = fs::write(&paths.cancel, "");
    for _ in 0..40 {
        if paths.status.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    paths.status.exists()
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
fn exit_status_from_code(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;

    ExitStatus::from_raw(code << 8)
}

#[cfg(windows)]
fn exit_status_from_code(code: i32) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;

    ExitStatus::from_raw(code as u32)
}

#[cfg(test)]
#[path = "ptyxis/tests.rs"]
mod tests;
