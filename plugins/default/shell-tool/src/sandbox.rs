//! Общий process-level sandbox/workdir режим для one-shot и unified shell.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use proteus_contracts::domain::EXEC_SHELL;
use serde_json::Value;

const SANDBOX_ENV: &str = "PROTEUS_SHELL_SANDBOX";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SandboxKind {
    Bwrap(PathBuf),
}

impl SandboxKind {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Bwrap(_) => "bwrap",
        }
    }

    pub(crate) fn executable(&self) -> &Path {
        match self {
            Self::Bwrap(path) => path,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SandboxAvailability {
    Available(PathBuf),
    Disabled,
    Missing,
}

/// Process-level sandbox mode. Shell execution is trusted/direct by default;
/// setting `PROTEUS_SHELL_SANDBOX=1` opts the whole process into the bwrap
/// workspace sandbox. Detection is kept separate from selection so both shell
/// frontends can test it without mutating process environment variables.
#[derive(Clone, Debug)]
pub(crate) struct SandboxMode {
    availability: SandboxAvailability,
}

impl SandboxMode {
    pub(crate) fn detect(workspace: &str) -> Self {
        let setting = std::env::var(SANDBOX_ENV).ok();
        let availability = if sandbox_requested(setting.as_deref()) {
            sandbox_availability(workspace)
        } else {
            SandboxAvailability::Disabled
        };
        Self { availability }
    }

    pub(crate) fn enabled(&self) -> bool {
        !matches!(self.availability, SandboxAvailability::Disabled)
    }

    pub(crate) fn select(&self, uses_external_terminal: bool) -> Result<Option<SandboxKind>> {
        if !self.enabled() {
            return Ok(None);
        }
        if uses_external_terminal {
            return Err(anyhow!(
                "external terminal execution is unavailable while {SANDBOX_ENV}=1"
            ));
        }
        match &self.availability {
            SandboxAvailability::Available(executable) => {
                Ok(Some(SandboxKind::Bwrap(executable.clone())))
            }
            SandboxAvailability::Disabled => Ok(None),
            SandboxAvailability::Missing => Err(anyhow!(
                "{SANDBOX_ENV}=1 requires executable 'bwrap' in PATH"
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn enabled_unavailable_for_test() -> Self {
        Self {
            availability: SandboxAvailability::Missing,
        }
    }

    #[cfg(test)]
    pub(crate) fn disabled_for_test() -> Self {
        Self {
            availability: SandboxAvailability::Disabled,
        }
    }

    #[cfg(test)]
    pub(crate) fn enabled_for_workspace_test(workspace: &str) -> Self {
        Self {
            availability: sandbox_availability(workspace),
        }
    }
}

fn sandbox_requested(value: Option<&str>) -> bool {
    value == Some("1")
}

fn sandbox_availability(workspace: &str) -> SandboxAvailability {
    trusted_executable_in_path("bwrap", workspace)
        .map_or(SandboxAvailability::Missing, SandboxAvailability::Available)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    path.is_file()
        && path
            .metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn trusted_executable_in_path(name: &str, workspace: &str) -> Option<PathBuf> {
    let canonical_workspace = Path::new(workspace).canonicalize().ok()?;
    std::env::var_os("PATH").and_then(|path| {
        trusted_executable_in_directories(name, &canonical_workspace, std::env::split_paths(&path))
    })
}

fn trusted_executable_in_directories(
    name: &str,
    canonical_workspace: &Path,
    directories: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    directories.into_iter().find_map(|directory| {
        let candidate = directory.join(name);
        if !is_executable(&candidate) {
            return None;
        }
        let canonical_candidate = candidate.canonicalize().ok()?;
        (!canonical_candidate.starts_with(canonical_workspace)).then_some(canonical_candidate)
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedWorkdir {
    pub(crate) workspace: String,
    pub(crate) workdir: String,
}

/// Резолвит `workdir` и при включённом process-level sandbox проверяет его по
/// canonical paths, чтобы `..` и symlink не позволяли выйти из workspace.
pub(crate) fn resolve_workdir(
    cwd: &str,
    workdir: Option<&Value>,
    workspace_only: bool,
) -> Result<ResolvedWorkdir> {
    let requested = match workdir {
        None => Path::new(cwd).to_path_buf(),
        Some(value) => {
            let workdir = value
                .as_str()
                .ok_or_else(|| anyhow!("shell arg 'workdir' must be a string"))?;
            if workdir.trim().is_empty() {
                Path::new(cwd).to_path_buf()
            } else {
                let path = Path::new(workdir);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    Path::new(cwd).join(path)
                }
            }
        }
    };
    if !requested.is_dir() {
        return Err(anyhow!(
            "shell workdir does not exist or is not a directory: {}",
            requested.display()
        ));
    }

    let canonical_workspace = Path::new(cwd)
        .canonicalize()
        .map_err(|error| anyhow!("failed to resolve shell workspace {cwd}: {error}"))?;
    let canonical_workdir = requested.canonicalize().map_err(|error| {
        anyhow!(
            "failed to resolve shell workdir {}: {error}",
            requested.display()
        )
    })?;
    if workspace_only && !canonical_workdir.starts_with(&canonical_workspace) {
        return Err(anyhow!(
            "shell workdir is outside the workspace while {SANDBOX_ENV}=1: {}",
            requested.display()
        ));
    }
    Ok(ResolvedWorkdir {
        workspace: canonical_workspace.display().to_string(),
        workdir: canonical_workdir.display().to_string(),
    })
}

/// Env запускаемых команд: нейтрализует интерактивность (pagers), цвет и
/// локаль. Копия `UNIFIED_EXEC_ENV` из upstream Codex; брендовый маркер
/// `CODEX_CI` заменён на `PROTEUS_CI`.
pub(crate) const EXEC_COMMAND_ENV: [(&str, &str); 10] = [
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("LANG", "C.UTF-8"),
    ("LC_CTYPE", "C.UTF-8"),
    ("LC_ALL", "C.UTF-8"),
    ("COLORTERM", ""),
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GH_PAGER", "cat"),
    ("PROTEUS_CI", "1"),
];

/// argv для bwrap: read-only корень, единственный rw-bind workspace, без сети,
/// с отдельным PID namespace и свежими /dev,/proc,/tmp. Внешний workdir сюда
/// попасть не может: sandbox mode отклоняет его до spawn.
pub(crate) fn bwrap_args(command: &str, workspace: &str, workdir: &str) -> Vec<String> {
    vec![
        "--die-with-parent".to_owned(),
        "--unshare-net".to_owned(),
        "--unshare-pid".to_owned(),
        "--ro-bind".to_owned(),
        "/".to_owned(),
        "/".to_owned(),
        "--dev".to_owned(),
        "/dev".to_owned(),
        "--proc".to_owned(),
        "/proc".to_owned(),
        "--tmpfs".to_owned(),
        "/tmp".to_owned(),
        "--bind".to_owned(),
        workspace.to_owned(),
        workspace.to_owned(),
        "--chdir".to_owned(),
        workdir.to_owned(),
        EXEC_SHELL.to_owned(),
        "-lc".to_owned(),
        command.to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_one_enables_process_sandbox() {
        assert!(!sandbox_requested(None));
        assert!(!sandbox_requested(Some("0")));
        assert!(!sandbox_requested(Some("true")));
        assert!(sandbox_requested(Some("1")));
    }

    #[test]
    fn disabled_sandbox_selects_trusted_direct_execution() {
        assert_eq!(
            SandboxMode::disabled_for_test()
                .select(false)
                .expect("direct execution"),
            None
        );
    }

    #[test]
    fn external_terminal_is_allowed_only_when_sandbox_is_disabled() {
        assert_eq!(
            SandboxMode::disabled_for_test()
                .select(true)
                .expect("trusted external terminal"),
            None
        );
        let error = SandboxMode::enabled_unavailable_for_test()
            .select(true)
            .expect_err("sandbox mode must reject external terminal");
        assert!(error.to_string().contains("PROTEUS_SHELL_SANDBOX=1"));
    }

    #[test]
    fn external_workdir_is_allowed_only_when_workspace_limit_is_disabled() {
        let workspace = tempfile::tempdir().expect("workspace");
        let external = tempfile::tempdir().expect("external");
        let requested = Value::String(external.path().display().to_string());

        let error = resolve_workdir(
            &workspace.path().display().to_string(),
            Some(&requested),
            true,
        )
        .expect_err("sandbox mode must reject external workdir");
        assert!(error.to_string().contains("outside the workspace"));

        let resolved = resolve_workdir(
            &workspace.path().display().to_string(),
            Some(&requested),
            false,
        )
        .expect("trusted direct external workdir");
        assert_eq!(resolved.workdir, external.path().display().to_string());
    }

    #[cfg(unix)]
    #[test]
    fn sandboxed_workdir_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().expect("workspace");
        let external = tempfile::tempdir().expect("external");
        let link = workspace.path().join("outside");
        symlink(external.path(), &link).expect("symlink");
        let requested = Value::String(link.display().to_string());

        let error = resolve_workdir(
            &workspace.path().display().to_string(),
            Some(&requested),
            true,
        )
        .expect_err("sandbox mode must reject symlink escape");
        assert!(error.to_string().contains("outside the workspace"));
    }

    #[cfg(unix)]
    #[test]
    fn bwrap_candidate_inside_workspace_is_not_trusted() {
        use std::{fs, os::unix::fs::PermissionsExt as _};

        let workspace = tempfile::tempdir().expect("workspace");
        let candidate = workspace.path().join("bwrap");
        fs::write(&candidate, "#!/bin/sh\nexit 0\n").expect("shim");
        let mut permissions = fs::metadata(&candidate).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&candidate, permissions).expect("permissions");

        assert_eq!(
            trusted_executable_in_directories(
                "bwrap",
                workspace.path(),
                [workspace.path().to_path_buf()]
            ),
            None
        );
    }
}
