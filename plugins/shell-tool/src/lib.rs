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

use std::process::{Command, Stdio};

use agent_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginTool,
        PluginToolError, PluginToolObject, PluginTool_TO,
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

    fn invoke_json(
        &self,
        call_json: RString,
        cwd: RString,
    ) -> RResult<RString, PluginToolError> {
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

    let output = Command::new("sh")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| "failed to spawn shell")?;

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

    let error_msg = if !success {
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
