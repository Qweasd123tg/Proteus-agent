//! Direct `PatchApplier` reference process module.
//!
//! Registers patch applier id `"direct"` and applies the internal line-based
//! patch format inside the workspace passed by the host.

use std::path::{Path, PathBuf};

use proteus_contracts::{
    domain::{Patch, PatchResult},
    process_module::{ModuleRegistry, PatchModule, PatchModuleObject, ProcessModuleError},
};

mod transaction;

struct DirectPatchModule;

impl PatchModule for DirectPatchModule {
    fn apply_json(&self, patch_json: String, cwd: String) -> Result<String, ProcessModuleError> {
        let patch: Patch = match serde_json::from_str(patch_json.as_str()) {
            Ok(patch) => patch,
            Err(error) => {
                return Err(ProcessModuleError::new(format!(
                    "invalid Patch JSON: {error}"
                )));
            }
        };

        match apply_patch(&patch.content, Path::new(cwd.as_str())) {
            Ok(result) => match serde_json::to_string(&result) {
                Ok(json) => Ok(json),
                Err(error) => Err(ProcessModuleError::new(format!(
                    "failed to serialize PatchResult: {error}"
                ))),
            },
            Err(error) => Err(ProcessModuleError::new(error)),
        }
    }
}

fn apply_patch(input: &str, workspace_root: &Path) -> Result<PatchResult, String> {
    let operations = parse_patch(input)?;
    if operations.is_empty() {
        return Err("patch must contain at least one operation".to_owned());
    }

    let summaries = transaction::apply_operations(operations, workspace_root)?;

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
        if line == "@@" {
            parser.next();
            hunks.push(parse_hunk(parser)?);
            continue;
        }
        if line.starts_with("@@") {
            return Err(format!(
                "non-bare update hunk headers are unsupported; use bare '@@', got: {line}"
            ));
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

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let applier: PatchModuleObject = Box::new(DirectPatchModule);
    registry.register_patch(String::from("direct"), applier)
}

#[cfg(test)]
mod tests;
