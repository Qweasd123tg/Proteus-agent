//! Ripgrep implementations of the tool and context-provider contracts.

use std::{
    io::{BufRead, BufReader, Read},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, TryRecvError},
    time::{Duration, Instant},
};

use proteus_contracts::{
    domain::{ContextChunk, SearchQuery},
    process_module::{ModuleRegistry, ProcessModuleError},
};
use serde_json::{Value, json};

mod provider;
mod tool;
const RG_TIMEOUT: Duration = Duration::from_secs(60);

fn run_rg(query: SearchQuery) -> Result<Vec<ContextChunk>, String> {
    if query.text.trim().is_empty() || query.max_results == 0 {
        return Ok(Vec::new());
    }

    let command = build_rg_command(&query);
    let lines = match run_rg_limited(command, query, RG_TIMEOUT) {
        Ok(lines) => lines,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("ripgrep executable 'rg' was not found in PATH".to_owned());
        }
        Err(error) => return Err(format!("failed to run ripgrep: {error}")),
    };

    Ok(lines
        .iter()
        .map(String::as_str)
        .filter_map(parse_rg_match)
        .collect())
}

fn build_rg_command(query: &SearchQuery) -> Command {
    let mut command = Command::new("rg");
    command.arg("--json").arg("--max-filesize").arg("1M");
    // All suffixes form an OR predicate. If any literal cannot be narrowed
    // safely with a glob, scan all candidates and retain the exact postfilter.
    if let Some(globs) = query
        .ends_with
        .iter()
        .map(|suffix| suffix_glob(suffix))
        .collect::<Option<Vec<_>>>()
    {
        for glob in globs {
            command.arg("--glob").arg(glob);
        }
    }
    command.arg("--").arg(&query.text);
    for root in search_roots(query) {
        command.arg(root);
    }
    command.current_dir(&query.cwd).stdin(Stdio::null());
    command
}

fn search_roots(query: &SearchQuery) -> Vec<PathBuf> {
    let roots = query
        .starts_with
        .iter()
        .filter_map(|prefix| {
            let path = safe_relative_root(prefix)?;
            // A prefix is a predicate, not necessarily an existing path.
            // Only a trailing slash permits narrowing to that directory;
            // otherwise siblings with the same filename prefix also match.
            let mut root = if prefix.ends_with('/') {
                path.as_path()
            } else {
                path.parent()?
            };
            while !root.as_os_str().is_empty() && !query.cwd.join(root).is_dir() {
                root = root.parent()?;
            }
            Some(if root.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                root.to_path_buf()
            })
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if roots.is_empty() || roots.iter().any(|root| root == Path::new(".")) {
        vec![PathBuf::from(".")]
    } else {
        roots
            .iter()
            .filter(|root| {
                !roots
                    .iter()
                    .any(|parent| parent != *root && root.starts_with(parent))
            })
            .cloned()
            .collect()
    }
}

fn safe_relative_root(prefix: &str) -> Option<PathBuf> {
    let trimmed = prefix.trim_start_matches("./").trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return Some(PathBuf::from("."));
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return None;
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    Some(path.to_path_buf())
}

fn suffix_glob(suffix: &str) -> Option<String> {
    if suffix.is_empty()
        || suffix.contains("..")
        || suffix.contains('/')
        || suffix
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | ']' | '{' | '}' | '\\'))
    {
        return None;
    }
    Some(format!("*{suffix}"))
}

fn run_rg_limited(
    mut command: Command,
    query: SearchQuery,
    timeout: Duration,
) -> std::io::Result<Vec<String>> {
    let max_results = query.max_results;
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
        .ok_or_else(|| std::io::Error::other("failed to open rg stdout"))?;
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
            if parse_rg_match(&line).is_some_and(|chunk| {
                chunk
                    .path
                    .as_ref()
                    .and_then(|path| path.to_str())
                    .is_some_and(|path| query.matches_path(path))
            }) {
                lines.push(line);
                if lines.len() >= max_results {
                    let _ = tx.send(Ok((lines, true)));
                    return;
                }
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
                break Err(std::io::Error::other("rg stdout reader stopped"));
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

fn parse_rg_match(line: &str) -> Option<ContextChunk> {
    let event: Value = serde_json::from_str(line).ok()?;
    if event.get("type")?.as_str()? != "match" {
        return None;
    }
    let data = event.get("data")?;
    let path = normalize_rg_path(data.get("path")?.get("text")?.as_str()?);
    let line_number = data.get("line_number")?.as_u64()?;
    let line = data
        .get("lines")?
        .get("text")?
        .as_str()?
        .trim_end_matches(['\n', '\r']);
    let content = if line.chars().count() > 2000 {
        "[Omitted long matching line]".to_owned()
    } else {
        line.to_owned()
    };
    Some(
        ContextChunk::new("rg", content)
            .with_path(path.into())
            .with_metadata(json!({ "line": line_number })),
    )
}

fn normalize_rg_path(path: &str) -> &str {
    path.strip_prefix("./").unwrap_or(path)
}

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    registry.register_tool(Box::new(tool::SearchTool))?;
    registry.register_context_provider("rg_search".into(), Box::new(provider::SearchProvider))
}

#[cfg(test)]
mod tests;
