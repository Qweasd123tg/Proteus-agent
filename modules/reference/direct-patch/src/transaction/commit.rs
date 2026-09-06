use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

use super::{PlannedPath, checked_target_path, read_file_version};

pub(super) fn commit(
    workspace: &Path,
    paths: &BTreeMap<PathBuf, PlannedPath>,
) -> Result<(), String> {
    commit_with_hook(workspace, paths, |_| {})
}

fn commit_with_hook(
    workspace: &Path,
    paths: &BTreeMap<PathBuf, PlannedPath>,
    before_install: impl FnOnce(&[StagedChange]),
) -> Result<(), String> {
    let changed = paths
        .iter()
        .filter(|(_, state)| path_changed(state))
        .map(|(path, state)| (path.clone(), state.clone()))
        .collect::<Vec<_>>();
    if changed.is_empty() {
        return Ok(());
    }
    revalidate_originals(workspace, &changed)?;

    let transaction = tempfile::Builder::new()
        .prefix(".proteus-patch-")
        .tempdir_in(workspace)
        .map_err(|error| format!("failed to create patch transaction: {error}"))?;
    let mut changes = stage_changes(transaction.path(), changed)?;
    let mut created_dirs = Vec::new();
    for change in &changes {
        if change.staged.is_some()
            && let Err(error) = create_parent_dirs(
                workspace,
                change.target.parent().expect("patch target has a parent"),
                &mut created_dirs,
            )
        {
            let rollback = remove_created_dirs(&created_dirs);
            return Err(commit_error(error, rollback));
        }
    }

    for index in 0..changes.len() {
        if !changes[index].original_exists {
            continue;
        }
        if let Err(error) = fs::rename(&changes[index].target, &changes[index].backup) {
            let cause = format!(
                "failed to stage original {} for patch commit: {error}",
                changes[index].target.display()
            );
            let rollback = rollback_changes(&mut changes, &created_dirs);
            return Err(commit_failure(cause, rollback, transaction));
        }
        changes[index].backed_up = true;
    }

    before_install(&changes);
    for index in 0..changes.len() {
        let Some(staged) = changes[index].staged.as_ref() else {
            continue;
        };
        if let Err(error) = fs::rename(staged, &changes[index].target) {
            let cause = format!(
                "failed to install patched {}: {error}",
                changes[index].target.display()
            );
            let rollback = rollback_changes(&mut changes, &created_dirs);
            return Err(commit_failure(cause, rollback, transaction));
        }
        changes[index].installed = true;
    }
    Ok(())
}

struct StagedChange {
    target: PathBuf,
    staged: Option<PathBuf>,
    backup: PathBuf,
    original_exists: bool,
    backed_up: bool,
    installed: bool,
}

fn stage_changes(
    transaction_dir: &Path,
    changed: Vec<(PathBuf, PlannedPath)>,
) -> Result<Vec<StagedChange>, String> {
    changed
        .into_iter()
        .enumerate()
        .map(|(index, (target, state))| {
            let staged = if let Some(current) = state.current {
                let path = transaction_dir.join(format!("new-{index}"));
                fs::write(&path, current.content.as_bytes()).map_err(|error| {
                    format!("failed to stage patched {}: {error}", target.display())
                })?;
                if let Some(permissions) = current.permissions {
                    fs::set_permissions(&path, permissions).map_err(|error| {
                        format!(
                            "failed to preserve permissions for {}: {error}",
                            target.display()
                        )
                    })?;
                }
                Some(path)
            } else {
                None
            };
            Ok(StagedChange {
                target,
                staged,
                backup: transaction_dir.join(format!("old-{index}")),
                original_exists: state.original.is_some(),
                backed_up: false,
                installed: false,
            })
        })
        .collect()
}

fn rollback_changes(changes: &mut [StagedChange], created_dirs: &[PathBuf]) -> Vec<String> {
    let mut errors = Vec::new();
    for change in changes.iter_mut().rev().filter(|change| change.installed) {
        if let Err(error) = fs::remove_file(&change.target) {
            errors.push(format!(
                "failed to remove partial {}: {error}",
                change.target.display()
            ));
        } else {
            change.installed = false;
        }
    }
    for change in changes.iter_mut().rev().filter(|change| change.backed_up) {
        if let Err(error) = fs::rename(&change.backup, &change.target) {
            errors.push(format!(
                "failed to restore {}: {error}",
                change.target.display()
            ));
        } else {
            change.backed_up = false;
        }
    }
    errors.extend(remove_created_dirs(created_dirs));
    errors
}

fn commit_error(cause: String, rollback: Vec<String>) -> String {
    if rollback.is_empty() {
        format!("{cause}; workspace changes were rolled back")
    } else {
        format!(
            "{cause}; rollback incomplete: {}; workspace may be partially modified",
            rollback.join("; ")
        )
    }
}

fn commit_failure(cause: String, rollback: Vec<String>, transaction: tempfile::TempDir) -> String {
    if rollback.is_empty() {
        drop(transaction);
        format!("{cause}; workspace changes were rolled back")
    } else {
        let recovery_dir = transaction.keep();
        format!(
            "{cause}; rollback incomplete: {}; workspace may be partially modified; recovery files retained at {}",
            rollback.join("; "),
            recovery_dir.display()
        )
    }
}

fn remove_created_dirs(created_dirs: &[PathBuf]) -> Vec<String> {
    let mut errors = Vec::new();
    for directory in created_dirs.iter().rev() {
        if let Err(error) = fs::remove_dir(directory)
            && error.kind() != ErrorKind::NotFound
        {
            errors.push(format!(
                "failed to remove created directory {}: {error}",
                directory.display()
            ));
        }
    }
    errors
}

fn create_parent_dirs(
    workspace: &Path,
    parent: &Path,
    created: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let relative = parent
        .strip_prefix(workspace)
        .map_err(|_| format!("path escapes workspace: {}", parent.display()))?;
    let mut current = workspace.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(format!("path escapes workspace: {}", parent.display()));
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "refusing to create directory through symlink: {}",
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
            Err(error) if error.kind() == ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    format!("failed to create directory {}: {error}", current.display())
                })?;
                created.push(current.clone());
            }
            Err(error) => {
                return Err(format!("failed to inspect {}: {error}", current.display()));
            }
        }
    }
    Ok(())
}

fn revalidate_originals(
    workspace: &Path,
    changed: &[(PathBuf, PlannedPath)],
) -> Result<(), String> {
    for (target, state) in changed {
        let relative = target
            .strip_prefix(workspace)
            .map_err(|_| format!("path escapes workspace: {}", target.display()))?;
        checked_target_path(workspace, relative)?;
        let actual = read_file_version(target)?;
        let matches = match (&state.original, actual) {
            (None, None) => true,
            (Some(expected), Some(actual)) => expected.content == actual.content,
            _ => false,
        };
        if !matches {
            return Err(format!(
                "patch target changed during preflight: {}",
                target.display()
            ));
        }
    }
    Ok(())
}

fn path_changed(state: &PlannedPath) -> bool {
    match (&state.original, &state.current) {
        (None, None) => false,
        (Some(original), Some(current)) => {
            original.content != current.content
                || original.permissions.is_some() != current.permissions.is_some()
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::FileVersion;

    #[test]
    fn failed_install_rolls_back_already_installed_files() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = fs::canonicalize(dir.path()).unwrap();
        let first = workspace.join("a.txt");
        let second = workspace.join("b.txt");
        fs::write(&first, "old a\n").unwrap();
        fs::write(&second, "old b\n").unwrap();
        let mut paths = BTreeMap::new();
        for (path, old, new) in [
            (first.clone(), "old a\n", "new a\n"),
            (second.clone(), "old b\n", "new b\n"),
        ] {
            let permissions = fs::metadata(&path).unwrap().permissions();
            paths.insert(
                path,
                PlannedPath {
                    original: Some(FileVersion {
                        content: old.to_owned(),
                        permissions: Some(permissions.clone()),
                    }),
                    current: Some(FileVersion {
                        content: new.to_owned(),
                        permissions: Some(permissions),
                    }),
                },
            );
        }

        let error = commit_with_hook(&workspace, &paths, |changes| {
            fs::remove_file(changes[1].staged.as_ref().unwrap()).unwrap();
        })
        .unwrap_err();

        assert!(
            error.contains("workspace changes were rolled back"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(first).unwrap(), "old a\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "old b\n");
        assert!(fs::read_dir(&workspace).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".proteus-patch-")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn incomplete_rollback_retains_originals_for_manual_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = fs::canonicalize(dir.path()).unwrap();
        let first = workspace.join("a.txt");
        let second = workspace.join("b.txt");
        fs::write(&first, "old a\n").unwrap();
        fs::write(&second, "old b\n").unwrap();
        let mut paths = BTreeMap::new();
        for (path, old, new) in [
            (first.clone(), "old a\n", "new a\n"),
            (second.clone(), "old b\n", "new b\n"),
        ] {
            let permissions = fs::metadata(&path).unwrap().permissions();
            paths.insert(
                path,
                PlannedPath {
                    original: Some(FileVersion {
                        content: old.to_owned(),
                        permissions: Some(permissions.clone()),
                    }),
                    current: Some(FileVersion {
                        content: new.to_owned(),
                        permissions: Some(permissions),
                    }),
                },
            );
        }

        let error = commit_with_hook(&workspace, &paths, |changes| {
            let first_staged = changes[0].staged.as_ref().unwrap();
            fs::remove_file(first_staged).unwrap();
            fs::create_dir(first_staged).unwrap();
            fs::remove_file(changes[1].staged.as_ref().unwrap()).unwrap();
        })
        .unwrap_err();

        assert!(error.contains("rollback incomplete"), "{error}");
        assert!(error.contains("recovery files retained"), "{error}");
        assert_eq!(fs::read_to_string(&second).unwrap(), "old b\n");
        assert!(first.is_dir());
        let recovery = fs::read_dir(&workspace)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".proteus-patch-"))
            })
            .expect("recovery directory");
        assert_eq!(
            fs::read_to_string(recovery.join("old-0")).unwrap(),
            "old a\n"
        );
    }
}
