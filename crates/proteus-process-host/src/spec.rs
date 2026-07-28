use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    path::PathBuf,
    process::Command,
};

use anyhow::{Result, bail};

/// Minimal parent environment inherited by a new [`ProcessSpec`].
///
/// Unix children receive only `PATH` by default. Windows additionally needs
/// the standard process-launch and temporary-directory variables. Callers must
/// explicitly allow credentials and application-specific variables.
#[cfg(unix)]
pub const DEFAULT_ENV_ALLOWLIST: &[&str] = &["PATH"];

#[cfg(windows)]
pub const DEFAULT_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "WINDIR",
    "ComSpec",
    "TEMP",
    "TMP",
];

#[cfg(not(any(unix, windows)))]
pub const DEFAULT_ENV_ALLOWLIST: &[&str] = &["PATH"];

/// Launch description for a persistent child process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSpec {
    pub command: String,
    pub args: Vec<String>,
    /// Parent environment variable names copied into the child.
    pub env_allowlist: BTreeSet<String>,
    /// Explicit child values. These override allowlisted parent values.
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
}

impl ProcessSpec {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            env_allowlist: DEFAULT_ENV_ALLOWLIST
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            env: BTreeMap::new(),
            cwd: None,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn envs(
        mut self,
        env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.env.extend(
            env.into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }

    /// Adds parent environment names that the child is allowed to inherit.
    pub fn env_allowlist(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.env_allowlist.extend(names.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Resolves the exact child environment after validating configured names
    /// and values. Callers using a process API other than `std::process` can
    /// reuse the same fail-closed environment rules instead of reimplementing
    /// it.
    pub fn resolved_environment(&self) -> Result<BTreeMap<String, OsString>> {
        for name in self.env_allowlist.iter().chain(self.env.keys()) {
            validate_env_name(name)?;
        }
        for (name, value) in &self.env {
            if value.contains('\0') {
                bail!("environment variable {name:?} contains a NUL byte");
            }
        }

        let mut resolved = BTreeMap::new();
        for name in &self.env_allowlist {
            if let Some(value) = env::var_os(name) {
                resolved.insert(name.clone(), value);
            }
        }
        for (name, value) in &self.env {
            resolved.insert(name.clone(), OsString::from(value));
        }
        Ok(resolved)
    }

    pub(crate) fn apply_environment(&self, command: &mut Command) -> Result<()> {
        command.env_clear();
        command.envs(self.resolved_environment()?);
        Ok(())
    }
}

fn validate_env_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(['=', '\0']) {
        bail!("invalid environment variable name {name:?}");
    }
    Ok(())
}
