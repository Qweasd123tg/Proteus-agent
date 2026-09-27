//! Общие утилиты для всех file-tools: workspace containment, парсинг аргументов,
//! сериализация результатов.

use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::mpsc::{self, TryRecvError},
    time::{Duration, Instant},
};

pub(crate) use proteus_contracts::tool_support::{
    err_result, module_error, ok_result, optional_positive_usize, optional_string_array,
    parse_call, parse_invocation_context, required_string, workspace_path,
    workspace_path_for_write,
};

pub(crate) fn run_lines_limited(
    mut command: Command,
    max_results: usize,
    timeout: Duration,
) -> std::io::Result<Vec<String>> {
    if max_results == 0 {
        return Ok(Vec::new());
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("failed to open command stdout"))?;
    let (tx, rx) = mpsc::channel();
    let stderr = child.stderr.take().expect("piped stderr");
    let stderr_reader = std::thread::spawn(move || {
        let mut reader = stderr;
        let mut captured = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            if captured.len() < 8192 {
                captured.extend_from_slice(&buffer[..count.min(8192 - captured.len())]);
            }
        }
        Ok::<_, std::io::Error>(captured)
    });
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut lines = Vec::new();
        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(error) => {
                    let _ = tx.send(Err(error));
                    return;
                }
            };
            lines.push(line);
            if lines.len() >= max_results {
                let _ = tx.send(Ok((lines, true)));
                return;
            }
        }
        let _ = tx.send(Ok((lines, false)));
    });

    let started = Instant::now();
    let mut pending_lines = None;
    let result = loop {
        match rx.try_recv() {
            Ok(Ok((lines, true))) => match child.try_wait()? {
                Some(status) if !status.success() && status.code() != Some(1) => {
                    break Err(std::io::Error::other(format!("rg exited with {status}")));
                }
                Some(_) => break Ok(lines),
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Ok(lines);
                }
            },
            Ok(Ok((lines, false))) => {
                pending_lines = Some(lines);
            }
            Ok(Err(error)) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error);
            }
            Err(TryRecvError::Disconnected) if pending_lines.is_none() => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(std::io::Error::other("command stdout reader stopped"));
            }
            Err(TryRecvError::Disconnected | TryRecvError::Empty) => {}
        }
        if let Some(status) = child.try_wait()?
            && let Some(lines) = pending_lines.take()
        {
            if status.success() || status.code() == Some(1) {
                break Ok(lines);
            }
            break Err(std::io::Error::other(format!("rg exited with {status}")));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            break Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "rg timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stderr = stderr_reader
        .join()
        .map_err(|_| std::io::Error::other("rg stderr reader panicked"))??;
    result.map_err(|error| {
        if stderr.is_empty() {
            error
        } else {
            std::io::Error::new(
                error.kind(),
                format!("{error}: {}", String::from_utf8_lossy(&stderr).trim()),
            )
        }
    })
}
