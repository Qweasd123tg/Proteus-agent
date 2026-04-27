use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;

use crate::{
    contracts::PatchApplier,
    domain::{Patch, PatchResult},
};

#[derive(Debug, Clone)]
pub struct DirectPatchApplier {
    workspace_root: PathBuf,
}

impl DirectPatchApplier {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl PatchApplier for DirectPatchApplier {
    async fn apply(&self, patch: Patch) -> Result<PatchResult> {
        let operations = parse_patch(&patch.content)?;
        if operations.is_empty() {
            bail!("patch must contain at least one operation");
        }

        let workspace = canonical_workspace(&self.workspace_root).await?;
        let mut summaries = Vec::with_capacity(operations.len());
        for operation in operations {
            summaries.push(apply_operation(&workspace, operation).await?);
        }

        Ok(PatchResult {
            ok: true,
            summary: summaries.join("; "),
        })
    }
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

fn parse_patch(input: &str) -> Result<Vec<PatchOperation>> {
    let mut parser = PatchParser::new(input);
    match parser.next() {
        Some("*** Begin Patch") => {}
        Some(line) => bail!("patch must start with '*** Begin Patch', got: {line}"),
        None => bail!("patch must not be empty"),
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
                    .ok_or_else(|| anyhow!("failed to parse add file header"))?;
                operations.push(parse_add_file(&mut parser, path)?);
            }
            Some(line) if line.starts_with("*** Update File: ") => {
                let path = parser
                    .next()
                    .and_then(|line| line.strip_prefix("*** Update File: "))
                    .ok_or_else(|| anyhow!("failed to parse update file header"))?;
                operations.push(parse_update_file(&mut parser, path)?);
            }
            Some(line) if line.starts_with("*** Delete File: ") => {
                let path = parser
                    .next()
                    .and_then(|line| line.strip_prefix("*** Delete File: "))
                    .ok_or_else(|| anyhow!("failed to parse delete file header"))?;
                operations.push(PatchOperation::Delete {
                    path: parse_patch_path(path)?,
                });
            }
            Some(line) => bail!("unsupported patch header: {line}"),
            None => bail!("patch missing '*** End Patch'"),
        }
    }

    while let Some(line) = parser.peek() {
        if !line.is_empty() {
            bail!("unexpected content after '*** End Patch': {line}");
        }
        parser.next();
    }

    Ok(operations)
}

fn parse_add_file(parser: &mut PatchParser<'_>, path: &str) -> Result<PatchOperation> {
    let mut lines = Vec::new();
    while let Some(line) = parser.peek() {
        if line == "*** End Patch" || is_file_header(line) {
            break;
        }
        let line = parser.next().expect("peeked line exists");
        let Some(content) = line.strip_prefix('+') else {
            bail!("add file lines must start with '+'");
        };
        lines.push(content.to_owned());
    }

    if lines.is_empty() {
        bail!("add file requires at least one '+' line");
    }

    Ok(PatchOperation::Add {
        path: parse_patch_path(path)?,
        lines,
    })
}

fn parse_update_file(parser: &mut PatchParser<'_>, path: &str) -> Result<PatchOperation> {
    let mut move_to = None;
    if let Some(line) = parser.peek() {
        if let Some(target) = line.strip_prefix("*** Move to: ") {
            parser.next();
            move_to = Some(parse_patch_path(target)?);
        }
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
        bail!("expected '@@' or next patch header, got: {line}");
    }

    if hunks.is_empty() && move_to.is_none() {
        bail!("update file requires at least one hunk or a move target");
    }

    Ok(PatchOperation::Update {
        path: parse_patch_path(path)?,
        move_to,
        hunks,
        no_newline_at_eof,
    })
}

fn parse_hunk(parser: &mut PatchParser<'_>) -> Result<Hunk> {
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
            .ok_or_else(|| anyhow!("empty line inside update hunk"))?;
        let text = chars.as_str().to_owned();
        match prefix {
            ' ' => lines.push(HunkLine::Context(text)),
            '-' => lines.push(HunkLine::Remove(text)),
            '+' => lines.push(HunkLine::Add(text)),
            _ => bail!("unsupported update hunk line: {line}"),
        }
    }

    if lines.is_empty() {
        bail!("update hunk must not be empty");
    }

    Ok(Hunk { lines })
}

fn is_file_header(line: &str) -> bool {
    line.starts_with("*** Add File: ")
        || line.starts_with("*** Update File: ")
        || line.starts_with("*** Delete File: ")
}

fn parse_patch_path(path: &str) -> Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        bail!("patch path must not be empty");
    }
    Ok(PathBuf::from(trimmed))
}

async fn apply_operation(workspace: &Path, operation: PatchOperation) -> Result<String> {
    match operation {
        PatchOperation::Add { path, lines } => {
            let target = writable_workspace_path(workspace, &path).await?;
            if tokio::fs::metadata(&target).await.is_ok() {
                bail!("cannot add file that already exists: {}", path.display());
            }
            tokio::fs::write(&target, render_text(&lines, true))
                .await
                .with_context(|| format!("failed to write {}", target.display()))?;
            Ok(format!("added {}", path.display()))
        }
        PatchOperation::Update {
            path,
            move_to,
            hunks,
            no_newline_at_eof,
        } => {
            let source = existing_workspace_path(workspace, &path).await?;
            let original = tokio::fs::read_to_string(&source)
                .await
                .with_context(|| format!("failed to read {}", source.display()))?;
            let updated = apply_hunks(&original, &hunks, no_newline_at_eof)?;
            let destination = match move_to.as_ref() {
                Some(target) => writable_workspace_path(workspace, target).await?,
                None => source.clone(),
            };

            if destination != source && tokio::fs::metadata(&destination).await.is_ok() {
                bail!("move target already exists: {}", destination.display());
            }

            tokio::fs::write(&destination, updated)
                .await
                .with_context(|| format!("failed to write {}", destination.display()))?;
            if destination != source {
                tokio::fs::remove_file(&source)
                    .await
                    .with_context(|| format!("failed to remove {}", source.display()))?;
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
            let target = existing_workspace_path(workspace, &path).await?;
            tokio::fs::remove_file(&target)
                .await
                .with_context(|| format!("failed to delete {}", target.display()))?;
            Ok(format!("deleted {}", path.display()))
        }
    }
}

fn apply_hunks(original: &str, hunks: &[Hunk], no_newline_at_eof: bool) -> Result<String> {
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
            bail!("update hunk must include at least one context or removed line");
        }

        let Some(position) = find_subsequence(&lines, &old_lines, cursor) else {
            bail!("failed to match update hunk against current file content");
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

async fn canonical_workspace(path: &Path) -> Result<PathBuf> {
    tokio::fs::canonicalize(path)
        .await
        .with_context(|| format!("failed to canonicalize cwd {}", path.display()))
}

async fn existing_workspace_path(workspace: &Path, path: &Path) -> Result<PathBuf> {
    let clean = clean_relative_path(path)?;
    let target = workspace.join(clean);
    let canonical = tokio::fs::canonicalize(&target)
        .await
        .with_context(|| format!("failed to canonicalize {}", target.display()))?;
    if !canonical.starts_with(workspace) {
        bail!("path escapes workspace: {}", path.display());
    }
    Ok(canonical)
}

async fn writable_workspace_path(workspace: &Path, path: &Path) -> Result<PathBuf> {
    let clean = clean_relative_path(path)?;
    let target = workspace.join(&clean);

    if let Ok(canonical_target) = tokio::fs::canonicalize(&target).await {
        if !canonical_target.starts_with(workspace) {
            bail!("path escapes workspace: {}", path.display());
        }
        return Ok(canonical_target);
    }

    let parent = target
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("write path has no parent: {}", path.display()))?;
    tokio::fs::create_dir_all(&parent)
        .await
        .with_context(|| format!("failed to create {}", parent.display()))?;
    let canonical_parent = tokio::fs::canonicalize(&parent)
        .await
        .with_context(|| format!("failed to canonicalize {}", parent.display()))?;
    if !canonical_parent.starts_with(workspace) {
        bail!("path escapes workspace: {}", path.display());
    }

    let file_name = target
        .file_name()
        .ok_or_else(|| anyhow!("write path must name a file: {}", path.display()))?;
    Ok(canonical_parent.join(file_name))
}

fn clean_relative_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        bail!("absolute patch paths are not allowed: {}", path.display());
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => bail!("path escapes workspace: {}", path.display()),
            Component::RootDir | Component::Prefix(_) => {
                bail!("absolute patch paths are not allowed: {}", path.display())
            }
        }
    }

    if clean.as_os_str().is_empty() {
        bail!("patch path must not be empty");
    }

    Ok(clean)
}
