//! Ripgrep SearchBackend plugin.
//!
//! Registers search backend id `"rg"` through the stable plugin ABI.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::mpsc::{self, TryRecvError},
    time::{Duration, Instant},
};

use agent_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    contracts::SearchQuery,
    domain::ContextChunk,
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginSearchBackend,
        PluginSearchBackend_TO, PluginSearchError, SearchBackendObject,
    },
};
use serde_json::json;

struct RgSearchPlugin;
const RG_TIMEOUT: Duration = Duration::from_secs(15);

impl PluginSearchBackend for RgSearchPlugin {
    fn search_json(&self, query_json: RString) -> RResult<RString, PluginSearchError> {
        let query: SearchQuery = match serde_json::from_str(query_json.as_str()) {
            Ok(query) => query,
            Err(error) => {
                return RResult::RErr(PluginSearchError::new(format!(
                    "invalid SearchQuery JSON: {error}"
                )));
            }
        };

        match run_rg(query) {
            Ok(chunks) => match serde_json::to_string(&chunks) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => RResult::RErr(PluginSearchError::new(format!(
                    "failed to serialize search chunks: {error}"
                ))),
            },
            Err(error) => RResult::RErr(PluginSearchError::new(error)),
        }
    }
}

fn run_rg(query: SearchQuery) -> Result<Vec<ContextChunk>, String> {
    if query.text.trim().is_empty() || query.max_results == 0 {
        return Ok(Vec::new());
    }

    let command = build_rg_command(&query);
    let lines = match run_rg_limited(command, query.max_results, RG_TIMEOUT) {
        Ok(lines) => lines,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("ripgrep executable 'rg' was not found in PATH".to_owned());
        }
        Err(error) => return Err(format!("failed to run ripgrep: {error}")),
    };

    Ok(lines
        .iter()
        .map(String::as_str)
        .filter_map(parse_rg_line)
        .filter(|chunk| {
            chunk
                .path
                .as_ref()
                .and_then(|path| path.to_str())
                .is_some_and(|path| query.matches_path(path))
        })
        .take(query.max_results)
        .collect())
}

fn build_rg_command(query: &SearchQuery) -> Command {
    let mut command = Command::new("rg");
    command
        .arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--max-columns")
        .arg("2000")
        .arg("--max-filesize")
        .arg("1M")
        .arg("--")
        .arg(&query.text)
        .arg(".")
        .current_dir(&query.cwd)
        .stdin(Stdio::null());
    command
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

fn parse_rg_line(line: &str) -> Option<ContextChunk> {
    let mut parts = line.splitn(3, ':');
    let path = normalize_rg_path(parts.next()?);
    let line_number = parts.next()?.parse::<usize>().ok()?;
    let content = parts.next()?.to_owned();
    Some(
        ContextChunk::new("rg", content)
            .with_path(path.into())
            .with_metadata(json!({ "line": line_number })),
    )
}

fn normalize_rg_path(path: &str) -> &str {
    path.strip_prefix("./").unwrap_or(path)
}

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let backend: SearchBackendObject =
        PluginSearchBackend_TO::from_value(RgSearchPlugin, TD_Opaque);
    registry.register_search_backend(RString::from("rg"), backend)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("rg-search"),
        description: RStr::from_str("Workspace SearchBackend backed by ripgrep"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn parse_rg_line_extracts_path_line_and_content() {
        let chunk = parse_rg_line("src/main.rs:42:let value = 1;").unwrap();

        assert_eq!(chunk.source, "rg");
        assert_eq!(chunk.path.unwrap().display().to_string(), "src/main.rs");
        assert_eq!(chunk.content, "let value = 1;");
        assert_eq!(chunk.metadata["line"], 42);
    }

    #[test]
    fn parse_rg_line_normalizes_current_dir_prefix() {
        let chunk = parse_rg_line("./src/main.rs:42:let value = 1;").unwrap();

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
