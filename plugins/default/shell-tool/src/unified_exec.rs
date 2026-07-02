//! unified_exec: `exec_command` / `write_stdin` — персистентные интерактивные
//! PTY-сессии в духе Codex.
//!
//! `exec_command` запускает команду в PTY и ждёт вывод до `yield_time_ms`;
//! если процесс ещё жив, модель получает Session ID и продолжает диалог через
//! `write_stdin` (в том числе Ctrl-C/Ctrl-D как "\u{3}"/"\u{4}"). Сессии
//! живут в глобальном store внутри dylib и умирают вместе с ядром; sandbox —
//! тот же bubblewrap, что и у one-shot `shell`, с той же эскалацией через
//! `with_escalated_permissions`.

use std::{
    collections::HashMap,
    io::{Read as _, Write as _},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{PluginTool, PluginToolError},
};
use serde_json::{Value, json};

use crate::{SandboxKind, bwrap_args, omitted_marker, resolve_workdir, sandbox_kind};

const DEFAULT_EXEC_YIELD_MS: u64 = 10_000;
const DEFAULT_WRITE_YIELD_MS: u64 = 250;
const MIN_YIELD_MS: u64 = 250;
/// Пустой `chars` — это poll; заставляем модель ждать заметное время вместо
/// busy-loop из коротких пустых вызовов (Codex-семантика).
const MIN_EMPTY_WRITE_YIELD_MS: u64 = 5_000;
const MAX_YIELD_MS: u64 = 30_000;
/// Спековый timeout: max yield + запас на spawn/drain.
const SPEC_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 10_000;
const MAX_OUTPUT_TOKENS: u64 = 25_000;
const APPROX_BYTES_PER_TOKEN: u64 = 4;
/// Непрочитанный вывод сессии между вызовами; старое выталкивается спереди.
const SESSION_BUFFER_LIMIT: usize = 1024 * 1024;
const MAX_SESSIONS: usize = 16;
/// После выхода процесса даём reader'у дочитать хвост из PTY.
const EXIT_DRAIN_GRACE: Duration = Duration::from_millis(150);

pub(crate) struct ExecCommandTool;

impl PluginTool for ExecCommandTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "exec_command",
            "description": "Runs a shell command (sh -lc) in an interactive PTY session. Waits up to `yield_time_ms` for output; if the process is still running, returns a Session ID for follow-up interaction via `write_stdin`. Commands run in the same sandbox as `shell` (no network, read-only outside the workspace) when available. Set `with_escalated_permissions: true` with a short `justification` to request an unsandboxed run (requires user approval). At most 16 live sessions: the least recently used one is killed to make room, so close finished sessions via write_stdin (Ctrl-C/Ctrl-D). Safety: RunsCommands.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "cmd": { "type": "string", "description": "Shell command to execute." },
                    "workdir": {
                        "type": "string",
                        "description": "Working directory for the command; relative paths resolve against the workspace root. Defaults to the workspace root."
                    },
                    "yield_time_ms": {
                        "type": "integer",
                        "description": "How long to wait (in milliseconds) for output before yielding; 250-30000, default 10000."
                    },
                    "max_output_tokens": {
                        "type": "integer",
                        "description": "Approximate cap on returned output tokens; excess is truncated in the middle."
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
                "required": ["cmd"]
            },
            "safety": "RunsCommands",
            "timeout_ms": SPEC_TIMEOUT_MS,
            "metadata": {
                "category": "terminal",
                "tags": ["terminal", "interactive", "session", "repl"],
                "aliases": ["interactive shell", "repl", "long-running command"]
            }
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, cwd: RString) -> RResult<RString, PluginToolError> {
        match exec_command_impl(call_json.as_str(), cwd.as_str()) {
            Ok(result_json) => RResult::ROk(RString::from(result_json)),
            Err(error) => RResult::RErr(PluginToolError::new(format!("{error:#}"))),
        }
    }
}

pub(crate) struct WriteStdinTool;

impl PluginTool for WriteStdinTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "write_stdin",
            "description": "Writes characters to a running exec_command session and returns output produced within `yield_time_ms`. Send \"\\u0003\" (Ctrl-C) to interrupt or \"\\u0004\" (Ctrl-D) to close stdin; empty `chars` polls for more output. Safety: RunsCommands.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "integer",
                        "description": "Session ID returned by exec_command while the process was still running."
                    },
                    "chars": {
                        "type": "string",
                        "description": "Bytes to write to stdin (may be empty to poll for output)."
                    },
                    "yield_time_ms": {
                        "type": "integer",
                        "description": "How long to wait (in milliseconds) for output before yielding; 250-30000, default 250. Empty polls wait at least 5000."
                    },
                    "max_output_tokens": {
                        "type": "integer",
                        "description": "Approximate cap on returned output tokens; excess is truncated in the middle."
                    }
                },
                "required": ["session_id"]
            },
            "safety": "RunsCommands",
            "timeout_ms": SPEC_TIMEOUT_MS,
            "metadata": {
                "category": "terminal",
                "tags": ["terminal", "interactive", "session", "stdin"],
                "aliases": ["send input", "interrupt process", "poll output"]
            }
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(&self, call_json: RString, _cwd: RString) -> RResult<RString, PluginToolError> {
        match write_stdin_impl(call_json.as_str()) {
            Ok(result_json) => RResult::ROk(RString::from(result_json)),
            Err(error) => RResult::RErr(PluginToolError::new(format!("{error:#}"))),
        }
    }
}

/// Состояние вывода сессии; reader/wait-потоки будят ожидающих через Condvar.
struct SessionOutput {
    buffer: Vec<u8>,
    dropped_bytes: usize,
    /// PTY master дочитан до EOF — вывода больше не будет.
    closed: bool,
    exited: bool,
    exit_code: Option<i32>,
}

struct ExecSession {
    output: Mutex<SessionOutput>,
    output_cond: Condvar,
    writer: Mutex<Box<dyn std::io::Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// Держит master-сторону PTY живой, пока сессия существует.
    _master: Mutex<Box<dyn MasterPty + Send>>,
    sandbox: Option<SandboxKind>,
    /// Для LRU-prune: обновляется при каждом обращении к сессии.
    last_used: Mutex<Instant>,
}

impl ExecSession {
    fn touch(&self) {
        *lock(&self.last_used) = Instant::now();
    }

    fn push_output(&self, chunk: &[u8]) {
        let mut output = lock(&self.output);
        output.buffer.extend_from_slice(chunk);
        if output.buffer.len() > SESSION_BUFFER_LIMIT {
            let excess = output.buffer.len() - SESSION_BUFFER_LIMIT;
            output.buffer.drain(..excess);
            output.dropped_bytes += excess;
        }
        drop(output);
        self.output_cond.notify_all();
    }

    fn mark_closed(&self) {
        lock(&self.output).closed = true;
        self.output_cond.notify_all();
    }

    fn mark_exited(&self, exit_code: Option<i32>) {
        let mut output = lock(&self.output);
        output.exited = true;
        output.exit_code = exit_code;
        drop(output);
        self.output_cond.notify_all();
    }
}

/// Снимок, который вызов забирает из сессии после ожидания.
struct Collected {
    bytes: Vec<u8>,
    dropped_bytes: usize,
    exited: bool,
    exit_code: Option<i32>,
}

type SessionMap = HashMap<i64, Arc<ExecSession>>;

fn sessions() -> &'static Mutex<SessionMap> {
    static SESSIONS: OnceLock<Mutex<SessionMap>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_SESSION_ID: AtomicI64 = AtomicI64::new(1001);

/// Mutex-и делят только потоки этого модуля; после паники внутри guard'а
/// данные всё ещё согласованны настолько, насколько это возможно — работаем
/// дальше вместо каскадного отказа tool'а.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn exec_command_impl(call_json: &str, cwd: &str) -> Result<String> {
    let call: Value =
        serde_json::from_str(call_json).with_context(|| "failed to parse ToolCall JSON")?;
    let call_id = call
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let args = call.get("args");
    let cmd = args
        .and_then(|args| args.get("cmd"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("exec_command requires string arg 'cmd'"))?;
    let workdir = resolve_workdir(cwd, args.and_then(|args| args.get("workdir")))?;
    let yield_time_ms = resolve_yield_time_ms(args, DEFAULT_EXEC_YIELD_MS);
    let max_output_bytes = resolve_max_output_bytes(args);
    let escalated = args
        .and_then(|args| args.get("with_escalated_permissions"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sandbox = if escalated { None } else { sandbox_kind(cwd) };

    let started = Instant::now();
    let (session_id, session) = spawn_session(cmd, cwd, &workdir, sandbox)?;
    let collected = wait_and_collect(&session, Duration::from_millis(yield_time_ms));
    let wall_time = started.elapsed();
    if collected.exited {
        lock(sessions()).remove(&session_id);
    }

    let metadata = json!({
        "yield_time_ms": yield_time_ms,
        "workdir": workdir,
        "sandbox": sandbox.map(SandboxKind::label),
        "escalated": escalated,
    });
    Ok(render_result(
        &call_id,
        session_id,
        collected,
        wall_time,
        max_output_bytes,
        metadata,
    ))
}

fn write_stdin_impl(call_json: &str) -> Result<String> {
    let call: Value =
        serde_json::from_str(call_json).with_context(|| "failed to parse ToolCall JSON")?;
    let call_id = call
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let args = call.get("args");
    let session_id = args
        .and_then(|args| args.get("session_id"))
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("write_stdin requires integer arg 'session_id'"))?;
    let chars = args
        .and_then(|args| args.get("chars"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut yield_time_ms = resolve_yield_time_ms(args, DEFAULT_WRITE_YIELD_MS);
    if chars.is_empty() {
        yield_time_ms = yield_time_ms.max(MIN_EMPTY_WRITE_YIELD_MS);
    }
    let max_output_bytes = resolve_max_output_bytes(args);

    let session = lock(sessions()).get(&session_id).cloned().ok_or_else(|| {
        anyhow!("unknown exec session {session_id}; the process may have already exited")
    })?;
    session.touch();

    let started = Instant::now();
    if !chars.is_empty() {
        let write_result = {
            let mut writer = lock(&session.writer);
            writer
                .write_all(chars.as_bytes())
                .and_then(|()| writer.flush())
        };
        // Гонка с выходом процесса: write в мёртвый PTY отдаёт EIO. Тогда
        // репортим exit ниже вместо ошибки записи.
        if let Err(error) = write_result
            && !lock(&session.output).exited
        {
            return Err(anyhow!(error).context("failed to write to session stdin"));
        }
    }
    let collected = wait_and_collect(&session, Duration::from_millis(yield_time_ms));
    let wall_time = started.elapsed();
    if collected.exited {
        lock(sessions()).remove(&session_id);
    }

    let metadata = json!({
        "yield_time_ms": yield_time_ms,
        "stdin_bytes": chars.len(),
        "sandbox": session.sandbox.map(SandboxKind::label),
    });
    Ok(render_result(
        &call_id,
        session_id,
        collected,
        wall_time,
        max_output_bytes,
        metadata,
    ))
}

fn resolve_yield_time_ms(args: Option<&Value>, default_ms: u64) -> u64 {
    args.and_then(|args| args.get("yield_time_ms"))
        .and_then(Value::as_u64)
        .map_or(default_ms, |requested| {
            requested.clamp(MIN_YIELD_MS, MAX_YIELD_MS)
        })
}

fn resolve_max_output_bytes(args: Option<&Value>) -> usize {
    let tokens = args
        .and_then(|args| args.get("max_output_tokens"))
        .and_then(Value::as_u64)
        .map_or(DEFAULT_MAX_OUTPUT_TOKENS, |requested| {
            requested.clamp(1, MAX_OUTPUT_TOKENS)
        });
    (tokens * APPROX_BYTES_PER_TOKEN) as usize
}

fn spawn_session(
    command: &str,
    workspace: &str,
    workdir: &str,
    sandbox: Option<SandboxKind>,
) -> Result<(i64, Arc<ExecSession>)> {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| anyhow!("failed to open PTY: {error}"))?;

    let mut builder = match sandbox {
        Some(SandboxKind::Bwrap) => {
            let mut builder = CommandBuilder::new("bwrap");
            builder.args(bwrap_args(command, workspace, workdir));
            builder
        }
        None => {
            let mut builder = CommandBuilder::new("sh");
            builder.args(["-lc", command]);
            builder
        }
    };
    builder.cwd(workdir);

    let mut child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| anyhow!("failed to spawn command in PTY: {error}"))?;
    let killer = child.clone_killer();
    // Slave-сторона нужна только процессу; наш дескриптор закрываем, чтобы
    // reader получил EOF после выхода команды.
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| anyhow!("failed to clone PTY reader: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| anyhow!("failed to take PTY writer: {error}"))?;

    let session = Arc::new(ExecSession {
        output: Mutex::new(SessionOutput {
            buffer: Vec::new(),
            dropped_bytes: 0,
            closed: false,
            exited: false,
            exit_code: None,
        }),
        output_cond: Condvar::new(),
        writer: Mutex::new(writer),
        killer: Mutex::new(killer),
        _master: Mutex::new(pair.master),
        sandbox,
        last_used: Mutex::new(Instant::now()),
    });

    let reader_session = Arc::clone(&session);
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(read) => reader_session.push_output(&buf[..read]),
            }
        }
        reader_session.mark_closed();
    });

    let wait_session = Arc::clone(&session);
    std::thread::spawn(move || {
        let exit_code = child.wait().ok().map(|status| status.exit_code() as i32);
        wait_session.mark_exited(exit_code);
    });

    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let mut sessions = lock(sessions());
    prune_session_if_needed(&mut sessions);
    sessions.insert(session_id, Arc::clone(&session));
    drop(sessions);
    Ok((session_id, session))
}

/// LRU-prune как в Codex: при заполнении store первыми выкидываются
/// завершённые сессии, затем самая давно не использованная (её процесс
/// убивается). Модель при обращении к вытесненной сессии получает
/// "unknown exec session".
fn prune_session_if_needed(sessions: &mut SessionMap) {
    if sessions.len() < MAX_SESSIONS {
        return;
    }
    let meta: Vec<(i64, Instant, bool)> = sessions
        .iter()
        .map(|(id, session)| (*id, *lock(&session.last_used), lock(&session.output).exited))
        .collect();
    let Some(victim_id) = session_to_prune(&meta) else {
        return;
    };
    if let Some(victim) = sessions.remove(&victim_id) {
        let _ = lock(&victim.killer).kill();
    }
}

/// Чистая политика выбора жертвы: сначала exited, затем самый старый
/// `last_used`; id — детерминированный tiebreaker.
fn session_to_prune(meta: &[(i64, Instant, bool)]) -> Option<i64> {
    meta.iter()
        .min_by_key(|(id, last_used, exited)| (!exited, *last_used, *id))
        .map(|(id, _, _)| *id)
}

/// Ждёт до дедлайна (или до exit + drain) и забирает накопленный вывод.
fn wait_and_collect(session: &ExecSession, yield_time: Duration) -> Collected {
    let deadline = Instant::now() + yield_time;
    let mut output = lock(&session.output);
    let mut exit_seen_at: Option<Instant> = None;
    loop {
        if output.exited {
            let seen = *exit_seen_at.get_or_insert_with(Instant::now);
            if output.closed || seen.elapsed() >= EXIT_DRAIN_GRACE {
                break;
            }
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let mut wait = deadline - now;
        if let Some(seen) = exit_seen_at {
            let grace_left = EXIT_DRAIN_GRACE.saturating_sub(seen.elapsed());
            wait = wait.min(grace_left.max(Duration::from_millis(1)));
        }
        let (guard, _timeout) = session
            .output_cond
            .wait_timeout(output, wait)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        output = guard;
    }
    Collected {
        bytes: std::mem::take(&mut output.buffer),
        dropped_bytes: std::mem::take(&mut output.dropped_bytes),
        exited: output.exited,
        exit_code: output.exit_code,
    }
}

fn render_result(
    call_id: &str,
    session_id: i64,
    collected: Collected,
    wall_time: Duration,
    max_output_bytes: usize,
    mut metadata: Value,
) -> String {
    let raw_text = String::from_utf8_lossy(&collected.bytes).into_owned();
    let (text, truncated) = truncate_head_tail(&raw_text, max_output_bytes);

    let mut sections = vec![format!("Wall time: {:.4} seconds", wall_time.as_secs_f64())];
    if collected.exited {
        match collected.exit_code {
            Some(code) => sections.push(format!("Exit code: {code}")),
            None => sections.push("Process terminated without exit code".to_owned()),
        }
    } else {
        sections.push(format!(
            "Session ID: {session_id} (process is still running; interact via write_stdin)"
        ));
    }
    if collected.dropped_bytes > 0 {
        sections.push(format!(
            "[{} bytes of earlier output were dropped from the session buffer]",
            collected.dropped_bytes
        ));
    }
    sections.push(format!("Output:\n{text}"));

    let (ok, error_msg) = if collected.exited {
        match collected.exit_code {
            Some(0) => (true, None),
            Some(code) => (false, Some(format!("process exited with code {code}"))),
            None => (false, Some("process terminated by signal".to_owned())),
        }
    } else {
        (true, None)
    };

    if let Some(map) = metadata.as_object_mut() {
        map.insert(
            "session_id".to_owned(),
            if collected.exited {
                Value::Null
            } else {
                json!(session_id)
            },
        );
        map.insert("exited".to_owned(), json!(collected.exited));
        map.insert("exit_code".to_owned(), json!(collected.exit_code));
        map.insert(
            "wall_time_seconds".to_owned(),
            json!(wall_time.as_secs_f64()),
        );
        map.insert("output_bytes".to_owned(), json!(collected.bytes.len()));
        map.insert("dropped_bytes".to_owned(), json!(collected.dropped_bytes));
        map.insert("truncated".to_owned(), json!(truncated));
    }

    json!({
        "call_id": call_id,
        "ok": ok,
        "output": sections.join("\n"),
        "content": [],
        "error": error_msg,
        "metadata": metadata
    })
    .to_string()
}

/// Head+tail усечение: начало и конец видны, середина вырезается — как в
/// Codex, чтобы модель видела и старт команды, и актуальный хвост.
fn truncate_head_tail(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_owned(), false);
    }
    let head_target = max_bytes / 2;
    let tail_target = max_bytes - head_target;
    let head_end = floor_char_boundary(text, head_target);
    let tail_start = ceil_char_boundary(text, text.len() - tail_target);
    let omitted = tail_start - head_end;
    (
        format!(
            "{}\n{}\n{}",
            &text[..head_end],
            omitted_marker(omitted, text.len()),
            &text[tail_start..]
        ),
        true,
    )
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn exec_command(cwd: &std::path::Path, args: Value) -> Value {
        let call = json!({ "id": "call_exec", "name": "exec_command", "args": args });
        let result =
            exec_command_impl(&call.to_string(), &cwd.display().to_string()).expect("invoke");
        serde_json::from_str(&result).expect("tool result")
    }

    fn write_stdin(args: Value) -> Value {
        let call = json!({ "id": "call_stdin", "name": "write_stdin", "args": args });
        let result = write_stdin_impl(&call.to_string()).expect("invoke");
        serde_json::from_str(&result).expect("tool result")
    }

    #[test]
    fn exec_command_reports_exit_for_quick_command() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = exec_command(dir.path(), json!({ "cmd": "printf marker42" }));

        assert_eq!(result["ok"], true);
        let output = result["output"].as_str().expect("output");
        assert!(output.contains("marker42"), "{output}");
        assert!(output.contains("Exit code: 0"), "{output}");
        assert_eq!(result["metadata"]["exited"], true);
        assert_eq!(result["metadata"]["exit_code"], 0);
        assert_eq!(result["metadata"]["session_id"], Value::Null);
    }

    #[test]
    fn exec_command_keeps_session_and_write_stdin_interacts() {
        let dir = tempfile::tempdir().expect("workspace");

        let started = exec_command(dir.path(), json!({ "cmd": "cat", "yield_time_ms": 300 }));
        assert_eq!(started["ok"], true);
        assert_eq!(started["metadata"]["exited"], false);
        let session_id = started["metadata"]["session_id"]
            .as_i64()
            .expect("session id");
        assert!(
            started["output"]
                .as_str()
                .expect("output")
                .contains(&format!("Session ID: {session_id}"))
        );

        let echoed = write_stdin(json!({
            "session_id": session_id,
            "chars": "hello\n",
            "yield_time_ms": 500
        }));
        assert_eq!(echoed["ok"], true);
        assert!(
            echoed["output"].as_str().expect("output").contains("hello"),
            "{echoed}"
        );

        // Ctrl-D закрывает stdin, cat выходит с кодом 0 и сессия исчезает.
        let finished = write_stdin(json!({
            "session_id": session_id,
            "chars": "\u{4}",
            "yield_time_ms": 5000
        }));
        assert_eq!(finished["metadata"]["exited"], true, "{finished}");
        assert_eq!(finished["metadata"]["exit_code"], 0);

        let call = json!({
            "id": "call_stdin",
            "name": "write_stdin",
            "args": { "session_id": session_id, "chars": "x" }
        });
        let error = write_stdin_impl(&call.to_string()).expect_err("session must be gone");
        assert!(
            error.to_string().contains("unknown exec session"),
            "{error}"
        );
    }

    #[test]
    fn write_stdin_rejects_unknown_session() {
        let call = json!({
            "id": "call_stdin",
            "name": "write_stdin",
            "args": { "session_id": -1 }
        });

        let error = write_stdin_impl(&call.to_string()).expect_err("unknown session must error");

        assert!(
            error.to_string().contains("unknown exec session"),
            "{error}"
        );
    }

    #[test]
    fn exec_command_requires_cmd_arg() {
        let dir = tempfile::tempdir().expect("workspace");
        let call = json!({ "id": "call_exec", "name": "exec_command", "args": {} });

        let error = exec_command_impl(&call.to_string(), &dir.path().display().to_string())
            .expect_err("missing cmd must error");

        assert!(error.to_string().contains("requires string arg 'cmd'"));
    }

    #[test]
    fn exec_command_truncates_output_head_and_tail() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = exec_command(
            dir.path(),
            json!({
                "cmd": "yes 0123456789 | head -c 60000",
                "max_output_tokens": 100
            }),
        );

        assert_eq!(result["metadata"]["truncated"], true);
        let output = result["output"].as_str().expect("output");
        assert!(output.contains("omitted"), "{output}");
    }

    #[test]
    fn exec_command_reports_nonzero_exit() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = exec_command(dir.path(), json!({ "cmd": "exit 7" }));

        assert_eq!(result["ok"], false);
        assert_eq!(result["error"], "process exited with code 7");
        assert_eq!(result["metadata"]["exit_code"], 7);
    }

    #[test]
    fn exec_command_clamps_yield_time() {
        let dir = tempfile::tempdir().expect("workspace");

        let result = exec_command(dir.path(), json!({ "cmd": "sleep 3", "yield_time_ms": 1 }));

        assert_eq!(result["metadata"]["yield_time_ms"], MIN_YIELD_MS);
        assert_eq!(result["metadata"]["exited"], false);
        let session_id = result["metadata"]["session_id"]
            .as_i64()
            .expect("session id");
        // Прибираем за собой, чтобы не упереться в MAX_SESSIONS в других тестах.
        write_stdin(json!({ "session_id": session_id, "chars": "\u{3}", "yield_time_ms": 5000 }));
    }

    #[test]
    fn prune_prefers_exited_then_oldest() {
        let now = Instant::now();
        let older = now - Duration::from_secs(60);
        // Живая старая vs живая свежая: выкидываем старую.
        assert_eq!(
            session_to_prune(&[(1, older, false), (2, now, false)]),
            Some(1)
        );
        // Завершённая свежая vs живая старая: сначала завершённая.
        assert_eq!(
            session_to_prune(&[(1, older, false), (2, now, true)]),
            Some(2)
        );
        assert_eq!(session_to_prune(&[]), None);
    }

    #[test]
    fn truncate_head_tail_respects_char_boundaries() {
        let text = "ёжик".repeat(100); // 800 байт, все символы двухбайтовые
        let (truncated, was_truncated) = truncate_head_tail(&text, 101);
        assert!(was_truncated);
        assert!(truncated.contains("omitted"));

        let (untouched, was_truncated) = truncate_head_tail("short", 100);
        assert!(!was_truncated);
        assert_eq!(untouched, "short");
    }
}
