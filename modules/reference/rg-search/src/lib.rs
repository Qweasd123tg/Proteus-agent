//! Ripgrep `SearchBackend` reference process module.
//!
//! The implementation is linked into the reference worker; the host only
//! sees the shared `search` process contract.

use std::{
    io::{BufRead, BufReader, Read},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, TryRecvError},
    time::{Duration, Instant},
};

use proteus_contracts::{
    contracts::SearchQuery,
    domain::ContextChunk,
    process_module::{ModuleRegistry, ProcessModuleError, SearchModule, SearchModuleObject},
};
use serde_json::{Value, json};

struct RgSearchModule;
const RG_TIMEOUT: Duration = Duration::from_secs(60);

impl SearchModule for RgSearchModule {
    fn search_json(&self, query_json: String) -> Result<String, ProcessModuleError> {
        let query: SearchQuery = match serde_json::from_str(query_json.as_str()) {
            Ok(query) => query,
            Err(error) => {
                return Err(ProcessModuleError::new(format!(
                    "invalid SearchQuery JSON: {error}"
                )));
            }
        };

        match run_rg(query) {
            Ok(chunks) => match serde_json::to_string(&chunks) {
                Ok(json) => Ok(String::from(json)),
                Err(error) => Err(ProcessModuleError::new(format!(
                    "failed to serialize search chunks: {error}"
                ))),
            },
            Err(error) => Err(ProcessModuleError::new(error)),
        }
    }
}

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
    for suffix in &query.ends_with {
        if let Some(glob) = suffix_glob(suffix) {
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
    let trimmed = suffix.trim().trim_start_matches("./");
    if trimmed.is_empty() || trimmed.contains("..") {
        return None;
    }
    Some(format!("*{trimmed}"))
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
    let backend: SearchModuleObject = Box::new(RgSearchModule);
    registry.register_search(String::from("rg"), backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn parse_rg_match_extracts_path_line_and_content() {
        let chunk = parse_rg_match(r#"{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"let value = 1;\n"},"line_number":42}}"#).unwrap();

        assert_eq!(chunk.source, "rg");
        assert_eq!(chunk.path.unwrap().display().to_string(), "src/main.rs");
        assert_eq!(chunk.content, "let value = 1;");
        assert_eq!(chunk.metadata["line"], 42);
    }

    #[test]
    fn parse_rg_match_normalizes_current_dir_prefix() {
        let chunk = parse_rg_match(r#"{"type":"match","data":{"path":{"text":"./src/main.rs"},"lines":{"text":"let value = 1;\n"},"line_number":42}}"#).unwrap();

        assert_eq!(chunk.path.unwrap().display().to_string(), "src/main.rs");
    }

    #[test]
    fn rg_command_searches_workspace_path_explicitly() {
        let query = SearchQuery::new("needle", std::path::PathBuf::from("/tmp/workspace"), 10);
        let command = build_rg_command(&query);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(args.last().map(String::as_str), Some("."));
        assert_eq!(
            command.get_current_dir(),
            Some(std::path::Path::new("/tmp/workspace"))
        );
    }

    #[test]
    fn rg_command_uses_safe_path_filters_as_search_roots_and_globs() {
        let dir = temp_workspace();
        fs::create_dir(dir.join("src")).unwrap();
        let query = SearchQuery::new("needle", dir.clone(), 10)
            .with_path_filters(["src/", "../outside", "/tmp"], [".rs", "../secret"]);
        let command = build_rg_command(&query);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(args.windows(2).any(|pair| pair == ["--glob", "*.rs"]));
        assert_eq!(args.last().map(String::as_str), Some("src"));
        assert!(!args.iter().any(|arg| arg == "../outside"));
        assert!(!args.iter().any(|arg| arg == "/tmp"));
        assert!(!args.iter().any(|arg| arg.contains("secret")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn run_rg_returns_matches_from_tiny_workspace() {
        let dir = temp_workspace();
        fs::write(dir.join("a.txt"), "hello needle\n").expect("write a.txt");
        fs::write(dir.join("b.txt"), "other\nneedle two\n").expect("write b.txt");

        let chunks = run_rg(SearchQuery::new("needle", dir.clone(), 10)).expect("rg search");
        let paths = chunks
            .iter()
            .map(|chunk| chunk.path.as_ref().unwrap().display().to_string())
            .collect::<Vec<_>>();

        assert_eq!(chunks.len(), 2);
        assert!(paths.contains(&"a.txt".to_owned()));
        assert!(paths.contains(&"b.txt".to_owned()));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn run_rg_honors_starts_with_without_scanning_other_roots() {
        let dir = temp_workspace();
        fs::create_dir_all(dir.join("src")).expect("create src");
        fs::create_dir_all(dir.join("docs")).expect("create docs");
        fs::write(dir.join("src/a.txt"), "hello needle\n").expect("write src/a.txt");
        fs::write(dir.join("docs/b.txt"), "needle in docs\n").expect("write docs/b.txt");

        let chunks = run_rg(
            SearchQuery::new("needle", dir.clone(), 10)
                .with_path_filters(["src/"], [] as [&str; 0]),
        )
        .expect("rg search");
        let paths = chunks
            .iter()
            .map(|chunk| chunk.path.as_ref().unwrap().display().to_string())
            .collect::<Vec<_>>();

        assert_eq!(paths, ["src/a.txt"]);

        fs::write(dir.join("src/another.txt"), "needle\n").unwrap();
        for prefix in ["src/a", "./src/a"] {
            let chunks = run_rg(
                SearchQuery::new("needle", dir.clone(), 2)
                    .with_path_filters([prefix], [] as [&str; 0]),
            )
            .unwrap();
            assert_eq!(chunks.len(), 2, "{prefix}");
            assert!(
                chunks
                    .iter()
                    .all(|chunk| chunk.path.as_ref().unwrap().starts_with("src"))
            );
        }
        assert_eq!(
            run_rg(
                SearchQuery::new("needle", dir.clone(), 10)
                    .with_path_filters(["src/", "src/a"], [] as [&str; 0])
            )
            .unwrap()
            .len(),
            2
        );
        // Existing directory name without '/' is still a prefix of siblings.
        fs::create_dir_all(dir.join("src-other")).unwrap();
        fs::write(dir.join("src-other/a.txt"), "needle\n").unwrap();
        assert_eq!(
            run_rg(
                SearchQuery::new("needle", dir.clone(), 10)
                    .with_path_filters(["src"], [] as [&str; 0])
            )
            .unwrap()
            .len(),
            3
        );
        assert!(
            run_rg(
                SearchQuery::new("needle", dir.clone(), 1)
                    .with_path_filters(["missing/"], [] as [&str; 0])
            )
            .unwrap()
            .is_empty()
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn run_rg_preserves_single_file_and_colon_paths() {
        let dir = temp_workspace();
        fs::write(dir.join("part:one.txt"), "needle here\n").unwrap();
        let chunks = run_rg(
            SearchQuery::new("needle", dir.clone(), 10)
                .with_path_filters(["part:one.txt"], [] as [&str; 0]),
        )
        .unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].path.as_ref().unwrap(), Path::new("part:one.txt"));
        assert_eq!(chunks[0].metadata["line"], 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn run_rg_reports_invalid_pattern_but_not_no_matches() {
        let dir = temp_workspace();
        fs::write(dir.join("a.txt"), "hello\n").unwrap();
        assert!(
            run_rg(SearchQuery::new("absent", dir.clone(), 10))
                .unwrap()
                .is_empty()
        );
        let error = run_rg(SearchQuery::new("[", dir.clone(), 10)).unwrap_err();
        assert!(error.contains("regex parse error"), "{error}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn run_rg_keeps_bounded_location_for_long_matching_line() {
        let dir = temp_workspace();
        fs::write(
            dir.join("long.txt"),
            format!("needle{}\n", "x".repeat(4000)),
        )
        .unwrap();
        let chunks = run_rg(SearchQuery::new("needle", dir.clone(), 10)).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].path.as_ref().unwrap(), Path::new("long.txt"));
        assert_eq!(chunks[0].metadata["line"], 1);
        assert_eq!(chunks[0].content, "[Omitted long matching line]");
        fs::remove_dir_all(dir).unwrap();
    }

    fn temp_workspace() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "agent-rg-search-test-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }
}
