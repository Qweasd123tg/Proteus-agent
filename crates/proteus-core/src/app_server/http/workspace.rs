//! Read-only workspace browsing for clients; independent of agent tool execution.
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use anyhow::{Result, anyhow, bail};
use serde::Serialize;

const MAX_ENTRIES: usize = 1000;
const MAX_BYTES: usize = 512 * 1024;

#[derive(Serialize)]
pub(super) struct WorkspaceEntry {
    name: String,
    path: String,
    kind: &'static str,
}

#[derive(Serialize)]
pub(super) struct WorkspaceListing {
    path: String,
    entries: Vec<WorkspaceEntry>,
    truncated: bool,
}

#[derive(Serialize)]
pub(super) struct WorkspaceFile {
    path: String,
    size: u64,
    kind: &'static str,
    text: Option<String>,
}

pub(super) fn query_path(query: Option<&str>) -> Result<String> {
    let mut path = None;
    for pair in query
        .unwrap_or_default()
        .split('&')
        .filter(|s| !s.is_empty())
    {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "path" if path.is_none() => {
                path = Some(super::sessions::percent_decode_query_value(value)?)
            }
            "session_dir" | "token" => {}
            _ => bail!("unknown or duplicate workspace query parameter: {key}"),
        }
    }
    Ok(path.unwrap_or_default())
}

fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        bail!("workspace path must be relative and cannot contain '..'");
    }
    let root = root.canonicalize()?;
    let resolved = root.join(path).canonicalize()?;
    if !resolved.starts_with(&root) {
        bail!("path is outside the session workspace");
    }
    Ok(resolved)
}

pub(super) fn list(root: &Path, relative: String) -> Result<WorkspaceListing> {
    let path = resolve(root, &relative)?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in fs::read_dir(path)? {
        if entries.len() == MAX_ENTRIES {
            truncated = true;
            break;
        }
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("workspace contains a non-UTF-8 filename"))?;
        let kind = entry.file_type()?;
        let kind = if kind.is_symlink() {
            "symlink"
        } else if kind.is_dir() {
            "directory"
        } else if kind.is_file() {
            "file"
        } else {
            "special"
        };
        let path = Path::new(&relative)
            .join(&name)
            .to_string_lossy()
            .into_owned();
        entries.push(WorkspaceEntry { name, path, kind });
    }
    entries.sort_by(|a, b| {
        (a.kind != "directory", a.name.to_lowercase())
            .cmp(&(b.kind != "directory", b.name.to_lowercase()))
    });
    Ok(WorkspaceListing {
        path: relative,
        entries,
        truncated,
    })
}

pub(super) fn read(root: &Path, relative: String) -> Result<WorkspaceFile> {
    let path = resolve(root, &relative)?;
    // Reject directories and special nodes before opening (e.g. a FIFO).
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        bail!("preview requires a regular file");
    }
    let size = metadata.len();
    let mut result = WorkspaceFile {
        path: relative,
        size,
        kind: "too_large",
        text: None,
    };
    if size > MAX_BYTES as u64 {
        return Ok(result);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Ok(result);
    }
    result.kind = "binary";
    if !bytes.contains(&0)
        && let Ok(text) = String::from_utf8(bytes)
    {
        result.kind = "text";
        result.text = Some(text);
    }
    Ok(result)
}
