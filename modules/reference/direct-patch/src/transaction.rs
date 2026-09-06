use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

use super::{Hunk, HunkLine, PatchOperation};

mod commit;

#[derive(Clone)]
pub(super) struct FileVersion {
    content: String,
    permissions: Option<fs::Permissions>,
}

#[derive(Clone)]
pub(super) struct PlannedPath {
    original: Option<FileVersion>,
    current: Option<FileVersion>,
}

struct PatchPlan {
    workspace: PathBuf,
    paths: BTreeMap<PathBuf, PlannedPath>,
    summaries: Vec<String>,
}

pub(super) fn apply_operations(
    operations: Vec<PatchOperation>,
    workspace_root: &Path,
) -> Result<Vec<String>, String> {
    let workspace = fs::canonicalize(workspace_root).map_err(|error| {
        format!(
            "failed to canonicalize cwd {}: {error}",
            workspace_root.display()
        )
    })?;
    let mut plan = PatchPlan {
        workspace,
        paths: BTreeMap::new(),
        summaries: Vec::with_capacity(operations.len()),
    };
    for operation in operations {
        plan.apply(operation)?;
    }
    plan.commit()?;
    Ok(plan.summaries)
}

impl PatchPlan {
    fn apply(&mut self, operation: PatchOperation) -> Result<(), String> {
        let summary = match operation {
            PatchOperation::Add { path, lines } => {
                let target = self.resolve(&path)?;
                if self.current(&target)?.is_some() {
                    return Err(format!(
                        "cannot add file that already exists: {}",
                        path.display()
                    ));
                }
                self.set_current(
                    target,
                    Some(FileVersion {
                        content: render_text(&lines, true),
                        permissions: None,
                    }),
                );
                format!("added {}", path.display())
            }
            PatchOperation::Update {
                path,
                move_to,
                hunks,
                no_newline_at_eof,
            } => {
                let source = self.resolve(&path)?;
                let mut updated = self
                    .current(&source)?
                    .ok_or_else(|| format!("cannot update missing file: {}", path.display()))?;
                updated.content = apply_hunks(&updated.content, &hunks, no_newline_at_eof)?;

                if let Some(move_to) = move_to {
                    let destination = self.resolve(&move_to)?;
                    if destination == source {
                        self.set_current(source, Some(updated));
                        format!("updated {}", path.display())
                    } else if self.current(&destination)?.is_some() {
                        return Err(format!("move target already exists: {}", move_to.display()));
                    } else {
                        self.set_current(source, None);
                        self.set_current(destination, Some(updated));
                        format!(
                            "updated {} and moved to {}",
                            path.display(),
                            move_to.display()
                        )
                    }
                } else {
                    self.set_current(source, Some(updated));
                    format!("updated {}", path.display())
                }
            }
            PatchOperation::Delete { path } => {
                let target = self.resolve(&path)?;
                if self.current(&target)?.is_none() {
                    return Err(format!("cannot delete missing file: {}", path.display()));
                }
                self.set_current(target, None);
                format!("deleted {}", path.display())
            }
        };
        self.summaries.push(summary);
        Ok(())
    }

    fn resolve(&mut self, relative: &Path) -> Result<PathBuf, String> {
        let target = checked_target_path(&self.workspace, relative)?;
        if !self.paths.contains_key(&target) {
            let original = read_file_version(&target)?;
            self.paths.insert(
                target.clone(),
                PlannedPath {
                    current: original.clone(),
                    original,
                },
            );
        }
        Ok(target)
    }

    fn current(&self, target: &Path) -> Result<Option<FileVersion>, String> {
        self.paths
            .get(target)
            .map(|state| state.current.clone())
            .ok_or_else(|| format!("unresolved patch path: {}", target.display()))
    }

    fn set_current(&mut self, target: PathBuf, current: Option<FileVersion>) {
        self.paths
            .get_mut(&target)
            .expect("patch path is resolved before mutation")
            .current = current;
    }

    fn commit(&self) -> Result<(), String> {
        commit::commit(&self.workspace, &self.paths)
    }
}

pub(super) fn read_file_version(path: &Path) -> Result<Option<FileVersion>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to operate on symlink path: {}",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!("patch target is not a file: {}", path.display()));
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Ok(Some(FileVersion {
        content,
        permissions: Some(metadata.permissions()),
    }))
}

pub(super) fn checked_target_path(workspace: &Path, path: &Path) -> Result<PathBuf, String> {
    let clean = clean_relative_path(path)?;
    let target = workspace.join(&clean);
    let mut current = workspace.to_path_buf();
    if let Some(parent) = clean.parent() {
        for component in parent.components() {
            let Component::Normal(part) = component else {
                return Err(format!("path escapes workspace: {}", path.display()));
            };
            current.push(part);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(format!(
                        "refusing to operate through symlink: {}",
                        current.display()
                    ));
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(format!(
                        "path parent is not a directory: {}",
                        current.display()
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("failed to inspect {}: {error}", current.display()));
                }
            }
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(&target)
        && metadata.file_type().is_symlink()
    {
        return Err(format!(
            "refusing to operate on symlink path: {}",
            path.display()
        ));
    }
    Ok(target)
}

fn clean_relative_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Err(format!(
            "absolute patch paths are not allowed: {}",
            path.display()
        ));
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("path escapes workspace: {}", path.display()));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "absolute patch paths are not allowed: {}",
                    path.display()
                ));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err("patch path must not be empty".to_owned());
    }
    Ok(clean)
}

fn apply_hunks(original: &str, hunks: &[Hunk], no_newline_at_eof: bool) -> Result<String, String> {
    let (mut lines, mut trailing_newline) = split_lines(original);
    let mut cursor = 0;
    for hunk in hunks {
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        for line in &hunk.lines {
            match line {
                HunkLine::Context(text) => {
                    old_lines.push(text.clone());
                    new_lines.push(text.clone());
                }
                HunkLine::Remove(text) => old_lines.push(text.clone()),
                HunkLine::Add(text) => new_lines.push(text.clone()),
            }
        }
        if old_lines.is_empty() {
            return Err("update hunk must include at least one context or removed line".to_owned());
        }
        let Some(position) = find_subsequence(&lines, &old_lines, cursor) else {
            return Err("failed to match update hunk against current file content".to_owned());
        };
        let new_len = new_lines.len();
        lines.splice(position..position + old_lines.len(), new_lines);
        cursor = position + new_len;
    }
    if no_newline_at_eof {
        trailing_newline = false;
    }
    Ok(render_text(&lines, trailing_newline))
}

fn split_lines(text: &str) -> (Vec<String>, bool) {
    let trailing_newline = text.ends_with('\n');
    let lines = if text.is_empty() {
        Vec::new()
    } else {
        text.split_terminator('\n').map(str::to_owned).collect()
    };
    (lines, trailing_newline)
}

fn find_subsequence(lines: &[String], needle: &[String], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start.min(lines.len()));
    }
    if needle.len() > lines.len() {
        return None;
    }
    let last_start = lines.len() - needle.len();
    (start..=last_start).find(|&index| lines[index..index + needle.len()] == needle[..])
}

fn render_text(lines: &[String], trailing_newline: bool) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut text = lines.join("\n");
    if trailing_newline {
        text.push('\n');
    }
    text
}
