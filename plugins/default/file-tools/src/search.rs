//! `grep` tool: поиск по содержимому файлов через ripgrep.
//!
//! Требует установленный `rg` в `$PATH`. Если нет — tool всё ещё виден,
//! но возвращает ошибку при вызове. Это feature не bug — пусть модель
//! видит осмысленное сообщение "rg is not installed" вместо того чтобы
//! плагин молчал.

use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc::{self, TryRecvError},
    time::{Duration, Instant},
};

use agent_contracts::abi_stable::std_types::{RResult, RString};
use agent_contracts::plugin::{PluginTool, PluginToolError};
use serde_json::{Value, json};

use crate::util::{
    err_result, ok_result, optional_positive_usize, parse_call, plugin_error, required_string,
    workspace_path,
};

pub struct GrepTool;
const RG_TIMEOUT: Duration = Duration::from_secs(15);

impl PluginTool for GrepTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "grep",
            "description": "Search for a regex pattern in workspace files using ripgrep. Returns lines that match.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for (ripgrep syntax)."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in, relative to workspace. Defaults to workspace root."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum matching lines to return. Defaults to 50."
                    }
                },
                "required": ["pattern"]
            },
            "safety": "ReadOnly",
            "timeout_ms": 15000,
            "metadata": null
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        let call = match parse_call(call_json.as_str()) {
            Ok(c) => c,
            Err(e) => return plugin_error(e),
        };

        let pattern = match required_string(&call.args, "pattern", &call.name) {
            Ok(p) => p.to_owned(),
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let cwd_path = Path::new(cwd.as_str());
        let search_path_arg = call.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let search_path = match workspace_path(cwd_path, Path::new(search_path_arg)) {
            Ok(p) => p,
            Err(e) => return err_result(&call.id, &call.name, e),
        };
        let max_results = match optional_positive_usize(&call.args, "max_results", &call.name) {
            Ok(m) => m.unwrap_or(50),
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
            .arg("1M")
            .arg("--")
            .arg(&pattern)
            .arg(&search_path)
            .stdin(Stdio::null());

        let lines = match run_rg_limited(command, max_results, RG_TIMEOUT) {
            Ok(lines) => lines,
            Err(e) => {
                return err_result(
                    &call.id,
                    &call.name,
                    format!("failed to run ripgrep: {e} (is 'rg' installed?)"),
                );
            }
        };

        let match_count = lines.len();
        let truncated = match_count >= max_results;

        let output_text = if lines.is_empty() {
            "(no matches)".to_owned()
        } else {
            lines.join("\n")
        };
        let metadata = json!({
            "pattern": pattern,
            "path": search_path.display().to_string(),
            "match_count": match_count,
            "max_results": max_results,
            "truncated": truncated,
        });
        ok_result(&call.id, &call.name, output_text, metadata)
    }
}

fn run_rg_limited(
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
        .ok_or_else(|| std::io::Error::other("failed to open rg stdout"))?;
    let (tx, rx) = mpsc::channel();
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
            Err(TryRecvError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::other("rg stdout reader stopped"));
            }
            Err(TryRecvError::Empty) => {}
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
                "rg timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
