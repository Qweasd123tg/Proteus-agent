use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::domain::SessionId;

const SESSION_METADATA_FILE: &str = "session.json";
const SESSION_SCHEMA_VERSION: u32 = 2;
const SHORT_SESSION_ID_MODULUS: u128 = 10_000_000_000;
const SHORT_SESSION_ID_LEN: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionDirectoryKind {
    ShortNumeric,
    Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResolvedSessionIdentity {
    pub session_id: SessionId,
    pub directory_kind: SessionDirectoryKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionMetadata {
    schema_version: u32,
    session_id: SessionId,
    // Schema v2 stored workspace here. The reversible parent directory is now
    // authoritative, but the field remains in the selected on-disk contract
    // so existing short sessions and newly written sessions use one shape.
    workspace_path: PathBuf,
}

pub(super) fn short_session_directory_name(session_id: SessionId) -> String {
    format!("{:010}", session_id.as_u128() % SHORT_SESSION_ID_MODULUS)
}

pub(super) fn validate_new_session_target(session_dir: &Path, session_id: SessionId) -> Result<()> {
    match std::fs::symlink_metadata(session_dir) {
        Ok(_) => {
            let existing = resolve_session_identity(session_dir)?;
            if existing.session_id != session_id {
                bail!(
                    "short session directory collision at {}: existing session {}, new session {}",
                    session_dir.display(),
                    existing.session_id,
                    session_id
                );
            }
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect session target {}", session_dir.display())),
    }
}

pub(super) fn resolve_session_identity(session_dir: &Path) -> Result<ResolvedSessionIdentity> {
    let filesystem_metadata = std::fs::metadata(session_dir).with_context(|| {
        format!(
            "failed to inspect session directory {}",
            session_dir.display()
        )
    })?;
    if !filesystem_metadata.is_dir() {
        bail!("session path is not a directory: {}", session_dir.display());
    }

    let name = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            anyhow!(
                "session directory must have a UTF-8 basename: {}",
                session_dir.display()
            )
        })?;

    if is_short_numeric_name(name) {
        let metadata = read_required_metadata(session_dir)?;
        return Ok(ResolvedSessionIdentity {
            session_id: metadata.session_id,
            directory_kind: SessionDirectoryKind::ShortNumeric,
        });
    }

    let session_id = name.parse::<SessionId>().with_context(|| {
        format!(
            "session directory basename must be a 10-digit id or UUID: {}",
            session_dir.display()
        )
    })?;
    if let Some(metadata) = read_optional_metadata(session_dir)?
        && metadata.session_id != session_id
    {
        bail!(
            "session metadata id {} does not match UUID directory basename {}: {}",
            metadata.session_id,
            session_id,
            session_dir.display()
        );
    }

    Ok(ResolvedSessionIdentity {
        session_id,
        directory_kind: SessionDirectoryKind::Uuid,
    })
}

pub(super) async fn ensure_writable_identity(
    session_dir: &Path,
    session_id: SessionId,
    directory_kind: SessionDirectoryKind,
    workspace_path: &Path,
    directory_was_created: bool,
) -> Result<()> {
    match directory_kind {
        SessionDirectoryKind::Uuid => {
            let resolved = resolve_session_identity(session_dir)?;
            if resolved.session_id != session_id {
                bail!(
                    "session identity changed at {}: expected {}, found {}",
                    session_dir.display(),
                    session_id,
                    resolved.session_id
                );
            }
            Ok(())
        }
        SessionDirectoryKind::ShortNumeric => {
            let metadata_path = metadata_path(session_dir);
            match tokio::fs::read_to_string(&metadata_path).await {
                Ok(content) => {
                    let metadata = parse_metadata(&metadata_path, &content)?;
                    if metadata.session_id != session_id {
                        bail!(
                            "short session directory collision at {}: metadata belongs to {}, attempted {}",
                            session_dir.display(),
                            metadata.session_id,
                            session_id
                        );
                    }
                    Ok(())
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    if !directory_was_created && directory_has_entries(session_dir).await? {
                        bail!(
                            "short session directory is missing required metadata and already contains data: {}",
                            session_dir.display()
                        );
                    }
                    write_metadata(session_dir, session_id, workspace_path).await
                }
                Err(error) => Err(error)
                    .with_context(|| format!("failed to read {}", metadata_path.display())),
            }
        }
    }
}

fn is_short_numeric_name(name: &str) -> bool {
    name.len() == SHORT_SESSION_ID_LEN && name.bytes().all(|byte| byte.is_ascii_digit())
}

fn metadata_path(session_dir: &Path) -> PathBuf {
    session_dir.join(SESSION_METADATA_FILE)
}

fn read_required_metadata(session_dir: &Path) -> Result<SessionMetadata> {
    read_optional_metadata(session_dir)?.ok_or_else(|| {
        anyhow!(
            "10-digit session directory requires metadata: {}",
            metadata_path(session_dir).display()
        )
    })
}

fn read_optional_metadata(session_dir: &Path) -> Result<Option<SessionMetadata>> {
    let path = metadata_path(session_dir);
    match std::fs::read_to_string(&path) {
        Ok(content) => parse_metadata(&path, &content).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn parse_metadata(path: &Path, content: &str) -> Result<SessionMetadata> {
    let metadata: SessionMetadata = serde_json::from_str(content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if metadata.schema_version != SESSION_SCHEMA_VERSION {
        bail!(
            "unsupported session schema_version {} in {}; expected {}",
            metadata.schema_version,
            path.display(),
            SESSION_SCHEMA_VERSION
        );
    }
    Ok(metadata)
}

async fn directory_has_entries(session_dir: &Path) -> Result<bool> {
    let mut entries = tokio::fs::read_dir(session_dir)
        .await
        .with_context(|| format!("failed to read {}", session_dir.display()))?;
    Ok(entries.next_entry().await?.is_some())
}

async fn write_metadata(
    session_dir: &Path,
    session_id: SessionId,
    workspace_path: &Path,
) -> Result<()> {
    let path = metadata_path(session_dir);
    let metadata = SessionMetadata {
        schema_version: SESSION_SCHEMA_VERSION,
        session_id,
        workspace_path: workspace_path.to_path_buf(),
    };
    let mut content = serde_json::to_vec_pretty(&metadata)?;
    content.push(b'\n');
    tokio::fs::write(&path, content)
        .await
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_hyphenated_and_simple_uuid_directories_without_metadata() {
        let root = tempfile::tempdir().expect("root");
        let session_id = crate::domain::new_session_id();
        let hyphenated = root.path().join(session_id.to_string());
        let simple = root.path().join(session_id.simple().to_string());
        std::fs::create_dir(&hyphenated).expect("hyphenated dir");
        std::fs::create_dir(&simple).expect("simple dir");

        let hyphenated_identity =
            resolve_session_identity(&hyphenated).expect("hyphenated identity");
        let simple_identity = resolve_session_identity(&simple).expect("simple identity");

        assert_eq!(hyphenated_identity.session_id, session_id);
        assert_eq!(simple_identity.session_id, session_id);
        assert_eq!(
            hyphenated_identity.directory_kind,
            SessionDirectoryKind::Uuid
        );
        assert_eq!(simple_identity.directory_kind, SessionDirectoryKind::Uuid);
    }

    #[tokio::test]
    async fn resolves_existing_schema_v2_short_directory() {
        let root = tempfile::tempdir().expect("root");
        let workspace = tempfile::tempdir().expect("workspace");
        let session_id = crate::domain::new_session_id();
        let session_dir = root.path().join(short_session_directory_name(session_id));
        std::fs::create_dir(&session_dir).expect("short dir");
        write_metadata(&session_dir, session_id, workspace.path())
            .await
            .expect("metadata");

        let identity = resolve_session_identity(&session_dir).expect("short identity");

        assert_eq!(identity.session_id, session_id);
        assert_eq!(identity.directory_kind, SessionDirectoryKind::ShortNumeric);
    }

    #[tokio::test]
    async fn uuid_directory_rejects_mismatched_metadata() {
        let root = tempfile::tempdir().expect("root");
        let workspace = tempfile::tempdir().expect("workspace");
        let basename_id = crate::domain::new_session_id();
        let metadata_id = crate::domain::new_session_id();
        let session_dir = root.path().join(basename_id.to_string());
        std::fs::create_dir(&session_dir).expect("UUID dir");
        write_metadata(&session_dir, metadata_id, workspace.path())
            .await
            .expect("metadata");

        let error = resolve_session_identity(&session_dir).expect_err("mismatch must fail");

        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn rejects_unknown_metadata_fields_and_schema_versions() {
        let root = tempfile::tempdir().expect("root");
        let session_id = crate::domain::new_session_id();
        let session_dir = root.path().join(short_session_directory_name(session_id));
        std::fs::create_dir(&session_dir).expect("short dir");
        let path = metadata_path(&session_dir);
        std::fs::write(
            &path,
            serde_json::json!({
                "schema_version": 99,
                "session_id": session_id,
                "workspace_path": root.path(),
            })
            .to_string(),
        )
        .expect("metadata");
        let version_error = resolve_session_identity(&session_dir).expect_err("version must fail");
        assert!(version_error.to_string().contains("schema_version"));

        std::fs::write(
            &path,
            serde_json::json!({
                "schema_version": SESSION_SCHEMA_VERSION,
                "session_id": session_id,
                "workspace_path": root.path(),
                "unexpected": true,
            })
            .to_string(),
        )
        .expect("metadata");
        let field_error = resolve_session_identity(&session_dir).expect_err("field must fail");
        assert!(format!("{field_error:#}").contains("unknown field `unexpected`"));
    }
}
