use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::Write,
    path::Path,
};

use anyhow::{Context, Result, bail};
use proteus_contracts::domain::SessionId;

const SESSION_WRITE_LOCK_FILE: &str = "journal.write.lock";

#[derive(Debug)]
pub(super) struct SessionWriteOwnership {
    _file: File,
}

impl SessionWriteOwnership {
    pub(super) fn acquire(session_dir: &Path, session_id: SessionId) -> Result<Self> {
        let path = session_dir.join(SESSION_WRITE_LOCK_FILE);
        reject_symlink(&path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("failed to open session writer lock {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let owner = active_owner_description(&path);
                bail!(
                    "session {session_id} already has an active writer in another process{owner}; lock: {}",
                    path.display()
                );
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| {
                    format!("failed to acquire session writer lock {}", path.display())
                });
            }
        }

        file.set_len(0)
            .with_context(|| format!("failed to reset session writer lock {}", path.display()))?;
        writeln!(file, "pid={}", std::process::id())
            .with_context(|| format!("failed to identify session writer in {}", path.display()))?;
        file.sync_data()
            .with_context(|| format!("failed to sync session writer lock {}", path.display()))?;
        Ok(Self { _file: file })
    }
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "session writer lock must not be a symlink: {}",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to inspect writer lock {}", path.display()))
        }
    }
}

fn active_owner_description(path: &Path) -> String {
    fs::read_to_string(path)
        .ok()
        .and_then(|owner| {
            let owner = owner.trim();
            (!owner.is_empty()).then(|| format!(" ({owner})"))
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_file_is_persistent_but_unlocked_after_owner_drop() {
        let session_dir = tempfile::tempdir().expect("session dir");
        let session_id = proteus_contracts::domain::new_session_id();

        let ownership = SessionWriteOwnership::acquire(session_dir.path(), session_id)
            .expect("first ownership");
        let error = SessionWriteOwnership::acquire(session_dir.path(), session_id)
            .expect_err("second handle must not acquire ownership");
        assert!(
            error
                .to_string()
                .contains("active writer in another process"),
            "{error:#}"
        );

        drop(ownership);
        SessionWriteOwnership::acquire(session_dir.path(), session_id)
            .expect("ownership after drop");
        assert!(session_dir.path().join(SESSION_WRITE_LOCK_FILE).is_file());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_lock_file_is_rejected() {
        let session_dir = tempfile::tempdir().expect("session dir");
        let target = session_dir.path().join("target");
        fs::write(&target, "not a lock").expect("target");
        std::os::unix::fs::symlink(&target, session_dir.path().join(SESSION_WRITE_LOCK_FILE))
            .expect("lock symlink");

        let error = SessionWriteOwnership::acquire(
            session_dir.path(),
            proteus_contracts::domain::new_session_id(),
        )
        .expect_err("symlink lock must fail closed");
        assert!(error.to_string().contains("must not be a symlink"));
    }
}
