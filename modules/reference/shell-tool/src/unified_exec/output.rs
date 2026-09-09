//! Bounded output collection, completion and model-visible rendering.
use super::{
    CANCELLATION_POLL_INTERVAL, EXIT_DRAIN_GRACE, SESSION_BUFFER_LIMIT, invocation_is_cancelled,
    session::{ExecSession, lock},
};
use crate::omitted_marker;
use anyhow::Result;
use proteus_contracts::process_module::ToolModuleHostMut;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(super) struct Collected {
    pub(super) bytes: Vec<u8>,
    pub(super) dropped_bytes: usize,
    pub(super) exited: bool,
    pub(super) exit_code: Option<i32>,
}

/// Ждёт до дедлайна (или до exit + drain) и забирает накопленный вывод.
pub(super) fn wait_and_collect(
    session: &ExecSession,
    yield_time: Duration,
    host: &mut ToolModuleHostMut<'_>,
) -> Result<Collected> {
    let deadline = Instant::now() + yield_time;
    let mut output = lock(&session.output);
    let mut exit_seen_at: Option<Instant> = None;
    loop {
        // A process host round trip must not block the local output readers.
        drop(output);
        let cancelled = invocation_is_cancelled(host)?;
        output = lock(&session.output);
        if cancelled {
            drop(output);
            session.kill();
            anyhow::bail!("tool invocation canceled");
        }
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
        wait = wait.min(CANCELLATION_POLL_INTERVAL);
        let (guard, _timeout) = session
            .output_cond
            .wait_timeout(output, wait)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        output = guard;
    }
    Ok(Collected {
        bytes: std::mem::take(&mut output.buffer),
        dropped_bytes: std::mem::take(&mut output.dropped_bytes),
        exited: output.exited,
        exit_code: output.exit_code,
    })
}

pub(super) fn render_result(
    call_id: &str,
    session_id: i64,
    collected: Collected,
    wall_time: Duration,
    max_output_bytes: usize,
    mut metadata: Value,
) -> String {
    let raw_text = if collected.dropped_bytes == 0 {
        String::from_utf8_lossy(&collected.bytes).into_owned()
    } else {
        let (head, tail) = collected.bytes.split_at(SESSION_BUFFER_LIMIT / 2);
        format!(
            "{}\n{}\n{}",
            String::from_utf8_lossy(head),
            omitted_marker(
                collected.dropped_bytes,
                collected
                    .bytes
                    .len()
                    .saturating_add(collected.dropped_bytes)
            ),
            String::from_utf8_lossy(tail)
        )
    };
    let (text, truncated) = truncate_head_tail(&raw_text, max_output_bytes);

    let mut sections = vec![format!("Wall time: {:.4} seconds", wall_time.as_secs_f64())];
    if collected.exited {
        match collected.exit_code {
            Some(code) => sections.push(format!("Process exited with code {code}")),
            None => sections.push("Process terminated without exit code".to_owned()),
        }
    } else {
        sections.push(format!("Process running with session ID {session_id}"));
    }
    if collected.dropped_bytes > 0 {
        sections.push(format!(
            "[{} bytes from the middle of output were dropped from the session buffer]",
            collected.dropped_bytes
        ));
    }
    sections.push(format!("Output:\n{text}"));

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
        map.insert(
            "truncated".to_owned(),
            json!(truncated || collected.dropped_bytes > 0),
        );
    }

    // Parity с upstream Codex (`ExecCommandToolOutput`): unified exec всегда
    // отдаёт success — exit code процесса это данные в тексте/metadata, а не
    // сбой tool-а. Иначе Ctrl-C или опрос умершей сессии выглядят для модели
    // как ошибка write_stdin (dogfood 2026-07-06: слепые повторы).
    json!({
        "call_id": call_id,
        "ok": true,
        "output": sections.join("\n"),
        "content": [],
        "error": Value::Null,
        "metadata": metadata
    })
    .to_string()
}

/// Head+tail усечение: начало и конец видны, середина вырезается — как в
/// Codex, чтобы модель видела и старт команды, и актуальный хвост.
pub(super) fn truncate_head_tail(text: &str, max_bytes: usize) -> (String, bool) {
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
