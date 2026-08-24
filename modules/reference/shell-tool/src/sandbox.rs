//! Общая sandbox/workdir policy для one-shot и unified shell execution.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

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
    Unavailable,
}

/// Process-level sandbox policy. Detection is kept separate from selection so
/// both shell frontends can regression-test fail-closed behavior without
/// mutating process environment variables in parallel tests.
#[derive(Clone, Debug)]
pub(crate) struct SandboxPolicy {
    availability: SandboxAvailability,
}

impl SandboxPolicy {
    /// Used when the caller has already selected explicit unsandboxed
    /// execution. This avoids probing `bwrap` for a path that will not use it.
    pub(crate) fn not_required() -> Self {
        Self {
            availability: SandboxAvailability::Disabled,
        }
    }

    pub(crate) fn detect(workspace: &str) -> Self {
        let availability = if std::env::var(SANDBOX_ENV).is_ok_and(|value| value == "0") {
            SandboxAvailability::Disabled
        } else {
            match trusted_executable_in_path("bwrap", workspace) {
                Some(executable) if bwrap_is_usable(&executable, workspace) => {
                    SandboxAvailability::Available(executable)
                }
                Some(_) => SandboxAvailability::Unavailable,
                None => SandboxAvailability::Missing,
            }
        };
        Self { availability }
    }

    pub(crate) fn select(
        &self,
        escalated: bool,
        uses_unsandboxed_backend: bool,
    ) -> Result<Option<SandboxKind>> {
        if escalated {
            return Ok(None);
        }
        if uses_unsandboxed_backend {
            return Err(anyhow!(
                "external terminal execution is unsandboxed and requires \
                 with_escalated_permissions=true"
            ));
        }
        match &self.availability {
            SandboxAvailability::Available(executable) => {
                Ok(Some(SandboxKind::Bwrap(executable.clone())))
            }
            SandboxAvailability::Disabled => Err(anyhow!(
                "sandboxed shell execution is disabled by {SANDBOX_ENV}=0; retry with \
                 with_escalated_permissions=true to request an unsandboxed run"
            )),
            SandboxAvailability::Missing => Err(anyhow!(
                "sandboxed shell execution requires executable 'bwrap' in PATH; retry with \
                 with_escalated_permissions=true to request an unsandboxed run"
            )),
            SandboxAvailability::Unavailable => Err(anyhow!(
                "sandboxed shell execution found 'bwrap', but it cannot establish the required \
                 isolation; retry with with_escalated_permissions=true to request an unsandboxed run"
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn unavailable_for_test() -> Self {
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
    pub(crate) fn unusable_for_test() -> Self {
        Self {
            availability: SandboxAvailability::Unavailable,
        }
    }
}

/// A `bwrap` binary is available only when it can create the same namespace
/// and mount layout used for an actual non-escalated command. Presence in PATH
/// alone is insufficient on hosts that disable unprivileged namespaces.
fn bwrap_is_usable(executable: &Path, workspace: &str) -> bool {
    let Ok(mut child) = Command::new(executable)
        .args(bwrap_args("true", workspace, workspace))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
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

/// Резолвит `workdir` и проверяет его по canonical paths, чтобы `..` и symlink
/// не позволяли неэскалированному вызову выйти из workspace.
pub(crate) fn resolve_workdir(
    cwd: &str,
    workdir: Option<&Value>,
    escalated: bool,
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
    if !escalated && !canonical_workdir.starts_with(&canonical_workspace) {
        return Err(anyhow!(
            "shell workdir is outside the workspace and requires \
             with_escalated_permissions=true: {}",
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
/// попасть не может: policy отклоняет его до spawn.
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
    fn disabled_sandbox_is_fail_closed() {
        let error = SandboxPolicy::disabled_for_test()
            .select(false, false)
            .expect_err("disabled sandbox must reject non-escalated execution");
        assert!(error.to_string().contains("PROTEUS_SHELL_SANDBOX=0"));
    }

    #[test]
    fn external_terminal_requires_escalation() {
        let error = SandboxPolicy::unusable_for_test()
            .select(false, true)
            .expect_err("external terminal must reject non-escalated execution");
        assert!(error.to_string().contains("external terminal"));
        assert_eq!(
            SandboxPolicy::unusable_for_test()
                .select(true, true)
                .expect("escalated external terminal"),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn bwrap_probe_rejects_an_executable_that_cannot_create_a_sandbox() {
        use std::{fs, os::unix::fs::PermissionsExt as _};

        let directory = tempfile::tempdir().expect("directory");
        let executable = directory.path().join("bwrap");
        fs::write(&executable, "#!/bin/sh\nexit 1\n").expect("shim");
        let mut permissions = fs::metadata(&executable).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("permissions");

        assert!(!bwrap_is_usable(
            &executable,
            directory.path().to_str().expect("workspace path")
        ));

        let error = SandboxPolicy::unusable_for_test()
            .select(false, false)
            .expect_err("unusable bwrap must fail closed");
        assert!(
            error
                .to_string()
                .contains("cannot establish the required isolation"),
            "{error}"
        );
        assert_eq!(
            SandboxPolicy::unusable_for_test()
                .select(true, false)
                .expect("escalated run bypasses sandbox"),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn bwrap_probe_is_bounded_when_the_executable_hangs() {
        use std::{fs, os::unix::fs::PermissionsExt as _};

        let directory = tempfile::tempdir().expect("directory");
        let executable = directory.path().join("bwrap");
        fs::write(&executable, "#!/bin/sh\nexec sleep 5\n").expect("shim");
        let mut permissions = fs::metadata(&executable).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("permissions");

        let started = Instant::now();
        assert!(!bwrap_is_usable(
            &executable,
            directory.path().to_str().expect("workspace path")
        ));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "bwrap probe was not bounded: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn external_workdir_requires_escalation() {
        let workspace = tempfile::tempdir().expect("workspace");
        let external = tempfile::tempdir().expect("external");
        let requested = Value::String(external.path().display().to_string());

        let error = resolve_workdir(
            &workspace.path().display().to_string(),
            Some(&requested),
            false,
        )
        .expect_err("external workdir must fail without escalation");
        assert!(error.to_string().contains("outside the workspace"));

        let resolved = resolve_workdir(
            &workspace.path().display().to_string(),
            Some(&requested),
            true,
        )
        .expect("escalated external workdir");
        assert_eq!(resolved.workdir, external.path().display().to_string());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_requires_escalation() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().expect("workspace");
        let external = tempfile::tempdir().expect("external");
        let link = workspace.path().join("outside");
        symlink(external.path(), &link).expect("symlink");
        let requested = Value::String(link.display().to_string());

        let error = resolve_workdir(
            &workspace.path().display().to_string(),
            Some(&requested),
            false,
        )
        .expect_err("symlink escape must fail without escalation");
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
