use std::{
    fs::{File, OpenOptions, TryLockError},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: String,
    pub expires_at: u64,
}

impl Credentials {
    pub fn validate(&self) -> Result<()> {
        if self.access_token.is_empty()
            || self.refresh_token.is_empty()
            || self.account_id.is_empty()
            || self.expires_at == 0
        {
            bail!("invalid ChatGPT credential file; sign in again");
        }
        Ok(())
    }
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

pub(super) async fn lock(path: &Path) -> Result<File> {
    std::fs::create_dir_all(parent(path)).context("create ChatGPT credential directory")?;
    // Separate stable inode: replacing the credential file must not replace
    // the lock. Closing the file releases the lock after crash/cancellation.
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    let lock_path = PathBuf::from(name);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(lock_path)
        .context("open ChatGPT credential lock")?;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => tokio::time::sleep(Duration::from_millis(25)).await,
            Err(TryLockError::Error(error)) => {
                return Err(error).context("lock ChatGPT credentials");
            }
        }
    }
}

pub(super) fn read(path: &Path) -> Result<Option<Credentials>> {
    let content = match std::fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("read ChatGPT credentials"),
    };
    let credentials: Credentials = serde_json::from_slice(&content)
        .context("invalid ChatGPT credential file; sign in again")?;
    credentials.validate()?;
    Ok(Some(credentials))
}

/// Caller holds `lock(path)` for the whole read/refresh/write transaction.
pub(super) fn write(path: &Path, credentials: &Credentials) -> Result<()> {
    credentials.validate()?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent(path))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(&serde_json::to_vec(credentials)?)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| e.error)
        .context("save ChatGPT credentials")?;
    #[cfg(unix)]
    File::open(parent(path))?.sync_all()?;
    Ok(())
}

pub(super) fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("remove ChatGPT credentials"),
    }
}
