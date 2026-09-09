//! Workspace path boundary retained from the existing patch module.
use std::{
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

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
