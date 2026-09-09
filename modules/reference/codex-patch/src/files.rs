// Adapted from OpenAI Codex 67cc3c318dc8b5532db6ade4182b1dc6f3870889.
// Apache-2.0; see ../UPSTREAM.md, ../LICENSE and ../NOTICE.
//! Verification precedes writes; application rereads files in hunk order.

use std::{collections::HashSet, fs, io::ErrorKind, path::Path};

use crate::{parser::Hunk, paths::checked_target_path, update};

fn source(hunk: &Hunk) -> &Path {
    match hunk {
        Hunk::AddFile { path, .. } | Hunk::DeleteFile { path } | Hunk::UpdateFile { path, .. } => {
            path
        }
    }
}

pub(super) fn verify(hunks: &[Hunk], workspace: &Path) -> Result<(), String> {
    let mut seen = HashSet::new();
    for hunk in hunks {
        let path = checked_target_path(workspace, source(hunk))?;
        if !seen.insert(path.clone()) {
            return Err(format!(
                "invalid patch: multiple operations target {}",
                path.display()
            ));
        }
        match hunk {
            Hunk::AddFile { .. } => {}
            Hunk::DeleteFile { .. } => {
                fs::read_to_string(&path)
                    .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
            }
            Hunk::UpdateFile {
                chunks, move_path, ..
            } => {
                updated(&path, chunks)?;
                if let Some(destination) = move_path {
                    checked_target_path(workspace, destination)?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn apply(hunks: &[Hunk], workspace: &Path) -> Result<String, String> {
    if hunks.is_empty() {
        return Err("No files were modified.".into());
    }
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    for hunk in hunks {
        let path = checked_target_path(workspace, source(hunk))?;
        match hunk {
            Hunk::AddFile { contents, .. } => {
                write(&path, contents)?;
                added.push(hunk.path());
            }
            Hunk::DeleteFile { .. } => {
                fs::remove_file(&path)
                    .map_err(|_| format!("Failed to delete file {}", path.display()))?;
                deleted.push(hunk.path());
            }
            Hunk::UpdateFile {
                chunks, move_path, ..
            } => {
                let contents = updated(&path, chunks)?;
                if let Some(destination) = move_path {
                    let destination = checked_target_path(workspace, destination)?;
                    write(&destination, &contents)?;
                    fs::remove_file(&path)
                        .map_err(|_| format!("Failed to remove original {}", path.display()))?;
                } else {
                    fs::write(&path, contents)
                        .map_err(|_| format!("Failed to write file {}", path.display()))?;
                }
                modified.push(hunk.path());
            }
        }
    }
    let mut summary = "Success. Updated the following files:\n".to_owned();
    for (kind, paths) in [('A', added), ('M', modified), ('D', deleted)] {
        for path in paths {
            summary.push_str(&format!("{kind} {}\n", path.display()));
        }
    }
    Ok(summary)
}

fn updated(path: &Path, chunks: &[crate::parser::UpdateFileChunk]) -> Result<String, String> {
    let original = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read file to update {}: {error}", path.display()))?;
    update::contents(&original, &path.display().to_string(), chunks)
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    match fs::write(path, contents) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|_| {
                    format!("Failed to create parent directories for {}", path.display())
                })?;
            }
            fs::write(path, contents)
                .map_err(|_| format!("Failed to write file {}", path.display()))
        }
        Err(_) => Err(format!("Failed to write file {}", path.display())),
    }
}
