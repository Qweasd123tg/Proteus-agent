//! Bounded, read-only Git views of the live session workspace.
use std::{
    ffi::OsStr,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use super::{MAX_BYTES, MAX_ENTRIES};

#[derive(Serialize)]
pub(crate) struct Changes {
    repository: bool,
    entries: Vec<Change>,
    truncated: bool,
}

#[derive(Serialize)]
struct Change {
    path: String,
    status: &'static str,
}

#[derive(Serialize)]
pub(crate) struct Diff {
    path: String,
    kind: &'static str,
    patch: Option<String>,
}

struct Output {
    success: bool,
    different: bool,
    bytes: Vec<u8>,
    truncated: bool,
}

fn git(root: &Path, args: &[&OsStr]) -> Result<Output> {
    let mut command = Command::new("git");
    command.current_dir(root).args([
        "--no-pager",
        "--literal-pathspecs",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "status.renames=true",
    ]);
    // The session cwd, rather than the server's inherited Git environment, owns scope.
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_PREFIX",
    ] {
        command.env_remove(name);
    }
    let mut child = command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("cannot start Git")?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("piped stdout")
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes);
    let truncated = bytes.len() > MAX_BYTES;
    if truncated || read.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    read?;
    bytes.truncate(MAX_BYTES);
    Ok(Output {
        success: status.success(),
        different: status.code() == Some(1),
        bytes,
        truncated,
    })
}

fn run(root: &Path, args: &[&str]) -> Result<Output> {
    git(root, &args.iter().map(OsStr::new).collect::<Vec<_>>())
}

fn repository(root: &Path) -> Result<bool> {
    let result = run(root, &["rev-parse", "--is-inside-work-tree"])?;
    Ok(result.success && result.bytes == b"true\n")
}

pub(crate) fn changes(root: &Path) -> Result<Changes> {
    let mut result = Changes {
        repository: repository(root)?,
        entries: Vec::new(),
        truncated: false,
    };
    if !result.repository {
        return Ok(result);
    }
    let prefix = run(root, &["rev-parse", "--show-prefix"])?;
    ensure!(
        prefix.success && !prefix.truncated,
        "cannot resolve Git workspace prefix"
    );
    let prefix = std::str::from_utf8(&prefix.bytes)?
        .strip_suffix('\n')
        .unwrap_or_default();
    let output = run(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=all",
            "--",
            ".",
        ],
    )?;
    ensure!(
        output.success || output.truncated,
        "cannot read Git changes"
    );
    result.truncated = output.truncated;
    // split_terminator alone would accept an incomplete final pathname at the cap.
    let complete = output
        .bytes
        .iter()
        .rposition(|b| *b == 0)
        .map_or(0, |i| i + 1);
    let mut records = output.bytes[..complete]
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty());
    while let Some(record) = records.next() {
        ensure!(
            record.len() >= 4 && record[2] == b' ',
            "invalid Git status record"
        );
        let xy = &record[..2];
        if (xy.contains(&b'R') || xy.contains(&b'C')) && records.next().is_none() {
            ensure!(output.truncated, "incomplete Git rename record");
            break;
        }
        let path = std::str::from_utf8(&record[3..]).context("Git path is not UTF-8")?;
        let Some(path) = path.strip_prefix(prefix) else {
            continue;
        };
        if result.entries.len() == MAX_ENTRIES {
            result.truncated = true;
            break;
        }
        let status = if xy.contains(&b'U') || xy == b"AA" || xy == b"DD" {
            "conflict"
        } else if xy == b"??" {
            "untracked"
        } else if xy.contains(&b'R') {
            "renamed"
        } else if xy.contains(&b'D') {
            "deleted"
        } else if xy.contains(&b'A') || xy.contains(&b'C') {
            "added"
        } else {
            "modified"
        };
        result.entries.push(Change {
            path: path.to_owned(),
            status,
        });
    }
    Ok(result)
}

// Deleted files have no canonical leaf. Check every existing ancestor and reject
// lexical traversal before Git receives the original, literal workspace path.
fn validate(root: &Path, relative: &str) -> Result<PathBuf> {
    ensure!(!relative.is_empty(), "diff requires a file path");
    let root = root.canonicalize()?;
    let mut path = root.clone();
    for part in Path::new(relative).components() {
        match part {
            Component::Normal(name) => path.push(name),
            Component::CurDir => continue,
            _ => bail!("workspace path must be relative and cannot contain '..'"),
        }
        match fs::symlink_metadata(&path) {
            Ok(_) => ensure!(
                path.canonicalize()?.starts_with(&root),
                "path is outside the session workspace"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    ensure!(path != root, "diff requires a file path");
    Ok(path)
}

pub(crate) fn diff(root: &Path, relative: String) -> Result<Diff> {
    let path = validate(root, &relative)?;
    let mut result = Diff {
        path: relative,
        kind: "unavailable",
        patch: None,
    };
    if !repository(root)? {
        return Ok(result);
    }
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Ok(result);
            }
            if metadata.len() > MAX_BYTES as u64 {
                result.kind = "too_large";
                return Ok(result);
            }
            Some(metadata)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let head = run(root, &["rev-parse", "--verify", "HEAD"])?;
    let tracked = run(root, &["ls-files", "--error-unmatch", "--", &result.path])?.success;
    let in_head = if head.success {
        !run(
            root,
            &["ls-tree", "--name-only", "HEAD", "--", &result.path],
        )?
        .bytes
        .is_empty()
    } else {
        false
    };
    let output = if head.success && (tracked || in_head) {
        run(
            root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-renames",
                "--relative",
                "HEAD",
                "--",
                &result.path,
            ],
        )?
    } else if metadata.is_some() {
        #[cfg(unix)]
        let null = "/dev/null";
        #[cfg(windows)]
        let null = "NUL";
        run(
            root,
            &[
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--",
                null,
                &format!("./{}", result.path),
            ],
        )?
    } else {
        return Ok(result);
    };
    if output.truncated {
        result.kind = "too_large";
        return Ok(result);
    }
    ensure!(output.success || output.different, "cannot read Git diff");
    result.kind = "binary";
    if let Ok(patch) = String::from_utf8(output.bytes)
        && !patch.contains('\0')
        && !patch
            .lines()
            .any(|line| line.starts_with("Binary files ") && line.ends_with(" differ"))
    {
        result.kind = "text";
        result.patch = Some(patch);
    }
    Ok(result)
}
