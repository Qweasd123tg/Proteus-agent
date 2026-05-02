//! Shell tool как dylib-плагин.
//!
//! Регистрирует один tool `shell` через `PluginTool` ABI. Безопасность
//! `RunsCommands` — `PermissionMode::Auto` запретит без approval, `plan`
//! скроет вообще. Вынесен из ядра именно ради этого: shell — самая
//! рискованная вещь, логично делать её opt-in через плагин, а не
//! встраивать.
//!
//! Реализация упрощена по сравнению с builtin, которая жила в
//! `modules/tools/shell.rs`: здесь нет зависимости от ядерных
//! `process_output` helpers, потому что плагин не может depend-ить на
//! `modular-agent`. Вместо сложного bounded-reader'а — простой cutoff
//! в 64 KB на stdout и stderr каждого (читается до конца, потом
//! обрезается). Этого хватает для базовых команд; для агрессивного
//! усечения лучше использовать команды с явным `| head`.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::{
    process::{Command, Output, Stdio},
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
use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

/// Максимум stdout/stderr в килобайтах. Агрессивнее чем ядерный bounded
/// reader — плагин не хочет таскать сложный I/O-слой, поэтому читает
/// всё в память и обрезает постфактум.
const OUTPUT_LIMIT_BYTES: usize = 64 * 1024;

/// Timeout на выполнение команды. Совпадает с builtin-значением из ядра,
/// чтобы замена была прозрачной.
const TIMEOUT_MS: u64 = 30_000;

struct ShellTool;

impl PluginTool for ShellTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "shell",
            "description": "Run a shell command in the current workspace (sh -lc). Safety: RunsCommands.",
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

    let child = Command::new("sh")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| "failed to spawn shell")?;
    let (output, timed_out) = wait_with_timeout(child, Duration::from_millis(TIMEOUT_MS))
        .with_context(|| "failed to wait for shell")?;

    let (stdout, stdout_truncated) = truncate(&output.stdout);
    let (stderr, stderr_truncated) = truncate(&output.stderr);
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
        "stdout_bytes": output.stdout.len(),
        "stderr_bytes": output.stderr.len(),
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated,
        "timed_out": timed_out,
        "timeout_ms": TIMEOUT_MS,
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

fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> std::io::Result<(Output, bool)> {
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            return Ok((output, false));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let output = child.wait_with_output()?;
            return Ok((output, true));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn truncate(buf: &[u8]) -> (String, bool) {
    if buf.len() <= OUTPUT_LIMIT_BYTES {
        return (String::from_utf8_lossy(buf).into_owned(), false);
    }
    let head = &buf[..OUTPUT_LIMIT_BYTES];
    (String::from_utf8_lossy(head).into_owned(), true)
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
}
