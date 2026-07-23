use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::domain::SessionId;

const SESSION_METADATA_FILE: &str = "session.json";
const SESSION_SCHEMA_VERSION: u32 = 3;
const JOURNAL_SCHEMA_VERSION: u32 = 1;
const SHORT_SESSION_ID_MODULUS: u128 = 10_000_000_000;
const SHORT_SESSION_ID_LEN: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionDirectoryKind {
    ShortNumeric,
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
    workspace_path: PathBuf,
    journal_schema_version: u32,
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

    if !is_short_numeric_name(name) {
        bail!(
            "session directory basename must be a 10-digit id for session schema v3: {}",
            session_dir.display()
        );
    }
    let metadata = read_required_metadata(session_dir)?;
    let expected_name = short_session_directory_name(metadata.session_id);
    if name != expected_name {
        bail!(
            "session directory basename {name} does not match session {} short id {expected_name}: {}",
            metadata.session_id,
            session_dir.display()
        );
    }
    Ok(ResolvedSessionIdentity {
        session_id: metadata.session_id,
        directory_kind: SessionDirectoryKind::ShortNumeric,
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
    let value: serde_json::Value = serde_json::from_str(content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let schema_version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            anyhow!(
                "session metadata in {} is missing schema_version",
                path.display()
            )
        })?;
    if schema_version != u64::from(SESSION_SCHEMA_VERSION) {
        bail!(
            "unsupported session schema_version {} in {}; expected {}",
            schema_version,
            path.display(),
            SESSION_SCHEMA_VERSION
        );
    }
    let metadata: SessionMetadata = serde_json::from_value(value)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if metadata.journal_schema_version != JOURNAL_SCHEMA_VERSION {
        bail!(
            "unsupported journal_schema_version {} in {}; expected {}",
            metadata.journal_schema_version,
            path.display(),
            JOURNAL_SCHEMA_VERSION
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
        journal_schema_version: JOURNAL_SCHEMA_VERSION,
    };
    let mut content = serde_json::to_vec_pretty(&metadata)?;
    content.push(b'\n');
    tokio::fs::write(&path, content)
        .await
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests;
