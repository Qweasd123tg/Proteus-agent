//! Direct PatchApplier plugin.
//!
//! Registers patch applier id `"direct"` and applies the internal line-based
//! patch format inside the workspace passed by the host.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use proteus_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    domain::{Patch, PatchResult},
    plugin::{
        PatchApplierObject, PluginPatchApplier, PluginPatchApplier_TO, PluginPatchError,
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref,
    },
};

struct DirectPatchPlugin;

impl PluginPatchApplier for DirectPatchPlugin {
    fn apply_json(&self, patch_json: RString, cwd: RString) -> RResult<RString, PluginPatchError> {
        let patch: Patch = match serde_json::from_str(patch_json.as_str()) {
            Ok(patch) => patch,
            Err(error) => {
                return RResult::RErr(PluginPatchError::new(format!(
                    "invalid Patch JSON: {error}"
                )));
            }
        };

        match apply_patch(&patch.content, Path::new(cwd.as_str())) {
            Ok(result) => match serde_json::to_string(&result) {
                Ok(json) => RResult::ROk(json.into()),
                Err(error) => RResult::RErr(PluginPatchError::new(format!(
                    "failed to serialize PatchResult: {error}"
                ))),
            },
            Err(error) => RResult::RErr(PluginPatchError::new(error)),
        }
    }
}

fn apply_patch(input: &str, workspace_root: &Path) -> Result<PatchResult, String> {
    let operations = parse_patch(input)?;
    if operations.is_empty() {
        return Err("patch must contain at least one operation".to_owned());
    }

    let workspace = canonical_workspace(workspace_root)?;
    let mut summaries = Vec::with_capacity(operations.len());
    for operation in operations {
        summaries.push(apply_operation(&workspace, operation)?);
    }

    Ok(PatchResult::new(true, summaries.join("; ")))
}

#[derive(Debug)]
enum PatchOperation {
    Add {
        path: PathBuf,
        lines: Vec<String>,
    },
    Update {
        path: PathBuf,
        move_to: Option<PathBuf>,
        hunks: Vec<Hunk>,
        no_newline_at_eof: bool,
    },
    Delete {
        path: PathBuf,
    },
}

#[derive(Debug)]
struct Hunk {
    lines: Vec<HunkLine>,
}

#[derive(Debug)]
enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

struct PatchParser<'a> {
    lines: Vec<&'a str>,
    index: usize,
}

impl<'a> PatchParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            lines: input
                .lines()
                .map(|line| line.strip_suffix('\r').unwrap_or(line))
                .collect(),
            index: 0,
        }
    }

    fn next(&mut self) -> Option<&'a str> {
        let line = self.lines.get(self.index).copied();
        if line.is_some() {
            self.index += 1;
        }
        line
    }

    fn peek(&self) -> Option<&'a str> {
        self.lines.get(self.index).copied()
    }
}

fn parse_patch(input: &str) -> Result<Vec<PatchOperation>, String> {
    let mut parser = PatchParser::new(input);
    match parser.next() {
        Some("*** Begin Patch") => {}
        Some(line) => {
            return Err(format!(
                "patch must start with '*** Begin Patch', got: {line}"
            ));
        }
        None => return Err("patch must not be empty".to_owned()),
    }

    let mut operations = Vec::new();
    loop {
        match parser.peek() {
            Some("*** End Patch") => {
                parser.next();
                break;
            }
            Some(line) if line.starts_with("*** Add File: ") => {
                let path = parser
                    .next()
                    .and_then(|line| line.strip_prefix("*** Add File: "))
                    .ok_or_else(|| "failed to parse add file header".to_owned())?;
                operations.push(parse_add_file(&mut parser, path)?);
            }
            Some(line) if line.starts_with("*** Update File: ") => {
                let path = parser
                    .next()
                    .and_then(|line| line.strip_prefix("*** Update File: "))
                    .ok_or_else(|| "failed to parse update file header".to_owned())?;
                operations.push(parse_update_file(&mut parser, path)?);
            }
            Some(line) if line.starts_with("*** Delete File: ") => {
                let path = parser
                    .next()
                    .and_then(|line| line.strip_prefix("*** Delete File: "))
                    .ok_or_else(|| "failed to parse delete file header".to_owned())?;
                operations.push(PatchOperation::Delete {
                    path: parse_patch_path(path)?,
                });
            }
            Some(line) => return Err(format!("unsupported patch header: {line}")),
            None => return Err("patch missing '*** End Patch'".to_owned()),
        }
    }

    while let Some(line) = parser.peek() {
        if !line.is_empty() {
            return Err(format!("unexpected content after '*** End Patch': {line}"));
        }
        parser.next();
    }

    Ok(operations)
}

fn parse_add_file(parser: &mut PatchParser<'_>, path: &str) -> Result<PatchOperation, String> {
    let mut lines = Vec::new();
    while let Some(line) = parser.peek() {
        if line == "*** End Patch" || is_file_header(line) {
            break;
        }
        let line = parser.next().expect("peeked line exists");
        let Some(content) = line.strip_prefix('+') else {
            return Err("add file lines must start with '+'".to_owned());
        };
        lines.push(content.to_owned());
    }

    if lines.is_empty() {
        return Err("add file requires at least one '+' line".to_owned());
    }

    Ok(PatchOperation::Add {
        path: parse_patch_path(path)?,
        lines,
    })
}

fn parse_update_file(parser: &mut PatchParser<'_>, path: &str) -> Result<PatchOperation, String> {
    let mut move_to = None;
    if let Some(line) = parser.peek()
        && let Some(target) = line.strip_prefix("*** Move to: ")
    {
        parser.next();
        move_to = Some(parse_patch_path(target)?);
    }

    let mut hunks = Vec::new();
    let mut no_newline_at_eof = false;
    while let Some(line) = parser.peek() {
        if line == "*** End Patch" || is_file_header(line) {
            break;
        }
        if line == "*** End of File" {
            no_newline_at_eof = true;
            parser.next();
            continue;
        }
        if line.starts_with("@@") {
            parser.next();
            hunks.push(parse_hunk(parser)?);
            continue;
        }
        return Err(format!("expected '@@' or next patch header, got: {line}"));
    }

    if hunks.is_empty() && move_to.is_none() {
        return Err("update file requires at least one hunk or a move target".to_owned());
    }

    Ok(PatchOperation::Update {
        path: parse_patch_path(path)?,
        move_to,
        hunks,
        no_newline_at_eof,
    })
}

fn parse_hunk(parser: &mut PatchParser<'_>) -> Result<Hunk, String> {
    let mut lines = Vec::new();
    while let Some(line) = parser.peek() {
        if line.starts_with("@@")
            || line == "*** End Patch"
            || line == "*** End of File"
            || is_file_header(line)
        {
            break;
        }

        let line = parser.next().expect("peeked line exists");
        let mut chars = line.chars();
        let prefix = chars
            .next()
            .ok_or_else(|| "empty line inside update hunk".to_owned())?;
        let text = chars.as_str().to_owned();
        match prefix {
            ' ' => lines.push(HunkLine::Context(text)),
            '-' => lines.push(HunkLine::Remove(text)),
            '+' => lines.push(HunkLine::Add(text)),
            _ => return Err(format!("unsupported update hunk line: {line}")),
        }
    }

    if lines.is_empty() {
        return Err("update hunk must not be empty".to_owned());
    }

    Ok(Hunk { lines })
}

fn is_file_header(line: &str) -> bool {
    line.starts_with("*** Add File: ")
        || line.starts_with("*** Update File: ")
        || line.starts_with("*** Delete File: ")
}

fn parse_patch_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("patch path must not be empty".to_owned());
    }
    Ok(PathBuf::from(trimmed))
}

fn apply_operation(workspace: &Path, operation: PatchOperation) -> Result<String, String> {
    match operation {
        PatchOperation::Add { path, lines } => {
            let target = writable_workspace_path(workspace, &path)?;
            if fs::metadata(&target).is_ok() {
                return Err(format!(
                    "cannot add file that already exists: {}",
                    path.display()
                ));
            }
            fs::write(&target, render_text(&lines, true))
                .map_err(|error| format!("failed to write {}: {error}", target.display()))?;
            Ok(format!("added {}", path.display()))
        }
        PatchOperation::Update {
            path,
            move_to,
            hunks,
            no_newline_at_eof,
        } => {
            let source = existing_workspace_path(workspace, &path)?;
            let original = fs::read_to_string(&source)
                .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
            let updated = apply_hunks(&original, &hunks, no_newline_at_eof)?;
            let destination = match move_to.as_ref() {
                Some(target) => writable_workspace_path(workspace, target)?,
                None => source.clone(),
            };

            if destination != source && fs::metadata(&destination).is_ok() {
                return Err(format!(
                    "move target already exists: {}",
                    destination.display()
                ));
            }

            fs::write(&destination, updated)
                .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
            if destination != source {
                fs::remove_file(&source)
                    .map_err(|error| format!("failed to remove {}: {error}", source.display()))?;
                return Ok(format!(
                    "updated {} and moved to {}",
                    path.display(),
                    move_to
                        .as_ref()
                        .expect("move target exists when destination differs")
                        .display()
                ));
            }

            Ok(format!("updated {}", path.display()))
        }
        PatchOperation::Delete { path } => {
            let target = existing_workspace_path(workspace, &path)?;
            fs::remove_file(&target)
                .map_err(|error| format!("failed to delete {}: {error}", target.display()))?;
            Ok(format!("deleted {}", path.display()))
        }
    }
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
        text.split_terminator('\n')
            .map(str::to_owned)
            .collect::<Vec<_>>()
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

fn canonical_workspace(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize cwd {}: {error}", path.display()))
}

fn existing_workspace_path(workspace: &Path, path: &Path) -> Result<PathBuf, String> {
    let clean = clean_relative_path(path)?;
    let target = workspace.join(clean);
    let canonical = fs::canonicalize(&target)
        .map_err(|error| format!("failed to canonicalize {}: {error}", target.display()))?;
    if !canonical.starts_with(workspace) {
        return Err(format!("path escapes workspace: {}", path.display()));
    }
    Ok(canonical)
}

fn writable_workspace_path(workspace: &Path, path: &Path) -> Result<PathBuf, String> {
    let clean = clean_relative_path(path)?;
    let target = workspace.join(&clean);

    if let Ok(canonical_target) = fs::canonicalize(&target) {
        if !canonical_target.starts_with(workspace) {
            return Err(format!("path escapes workspace: {}", path.display()));
        }
        return Ok(canonical_target);
    }

    let parent = target
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("write path has no parent: {}", path.display()))?;
    let canonical_parent = create_workspace_parent(workspace, &parent)
        .map_err(|error| format!("{error}: {}", path.display()))?;
    if !canonical_parent.starts_with(workspace) {
        return Err(format!("path escapes workspace: {}", path.display()));
    }

    let file_name = target
        .file_name()
        .ok_or_else(|| format!("write path must name a file: {}", path.display()))?;
    Ok(canonical_parent.join(file_name))
}

fn create_workspace_parent(workspace: &Path, parent: &Path) -> Result<PathBuf, String> {
    let relative = parent
        .strip_prefix(workspace)
        .map_err(|_| "path escapes workspace".to_owned())?;
    let mut current = workspace.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                current.push(part);
                if current.exists() {
                    let metadata = fs::symlink_metadata(&current).map_err(|error| {
                        format!("failed to inspect {}: {error}", current.display())
                    })?;
                    if metadata.file_type().is_symlink() {
                        return Err("path contains symlink ancestor".to_owned());
                    }
                    if !metadata.is_dir() {
                        return Err("path ancestor is not a directory".to_owned());
                    }
                    let canonical = fs::canonicalize(&current).map_err(|error| {
                        format!("failed to canonicalize {}: {error}", current.display())
                    })?;
                    if !canonical.starts_with(workspace) {
                        return Err("path escapes workspace".to_owned());
                    }
                } else {
                    fs::create_dir(&current).map_err(|error| {
                        format!("failed to create {}: {error}", current.display())
                    })?;
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("path escapes workspace".to_owned());
            }
        }
    }

    fs::canonicalize(&current)
        .map_err(|error| format!("failed to canonicalize {}: {error}", current.display()))
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

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let applier: PatchApplierObject =
        PluginPatchApplier_TO::from_value(DirectPatchPlugin, TD_Opaque);
    registry.register_patch_applier(RString::from("direct"), applier)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("direct-patch"),
        description: RStr::from_str("Workspace-scoped PatchApplier for internal patch format"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("sample.txt"), "hello modular agent\n").unwrap();
        dir
    }

    #[test]
    fn replaces_exact_text_once() {
        let dir = workspace();
        let result = apply_patch(
            "*** Begin Patch\n*** Update File: sample.txt\n@@\n-hello modular agent\n+patched modular agent\n*** End Patch",
            dir.path(),
        )
        .unwrap();

        assert!(result.ok);
        assert!(result.summary.contains("updated sample.txt"));
        assert_eq!(
            fs::read_to_string(dir.path().join("sample.txt")).unwrap(),
            "patched modular agent\n"
        );
    }

    #[test]
    fn adds_new_file_from_internal_format() {
        let dir = workspace();
        let result = apply_patch(
            "*** Begin Patch\n*** Add File: nested/new.txt\n+hello\n+patch\n*** End Patch",
            dir.path(),
        )
        .unwrap();

        assert!(result.ok);
        assert!(result.summary.contains("added nested/new.txt"));
        assert_eq!(
            fs::read_to_string(dir.path().join("nested").join("new.txt")).unwrap(),
            "hello\npatch\n"
        );
    }

    #[test]
    fn rejects_parent_traversal() {
        let dir = workspace();
        let error = apply_patch(
            "*** Begin Patch\n*** Add File: ../outside.txt\n+outside\n*** End Patch",
            dir.path(),
        )
        .unwrap_err();

        assert!(error.contains("escapes workspace"));
    }

    #[cfg(unix)]
    #[test]
    fn add_file_rejects_symlink_parent_without_creating_outside_dirs() {
        let dir = workspace();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();

        let error = apply_patch(
            "*** Begin Patch\n*** Add File: link/new/file.txt\n+outside\n*** End Patch",
            dir.path(),
        )
        .unwrap_err();

        assert!(error.contains("symlink ancestor"), "{error}");
        assert!(!outside.path().join("new").exists());
    }
}
