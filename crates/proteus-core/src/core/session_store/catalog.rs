use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result};
use proteus_contracts::app_protocol::AppSessionSummary;

use crate::{
    core::session_journal::{JournalProjection, journal_path, load_records},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

use super::{
    encode_workspace_path, identity::resolve_session_identity, workspace_path_from_session_dir,
};

#[derive(Clone, Copy)]
enum CatalogMode {
    Compatible,
    Audit,
}

pub fn list_session_summaries(config_root: &Path) -> Result<Vec<AppSessionSummary>> {
    list_all(config_root, CatalogMode::Compatible)
}

pub fn list_workspace_session_summaries(
    config_root: &Path,
    workspace_path: &Path,
) -> Result<Vec<AppSessionSummary>> {
    list_workspace(config_root, workspace_path, CatalogMode::Compatible)
}

/// Strict storage audit, including incompatible entries hidden from the UI catalog.
pub fn list_session_summaries_for_audit(
    config_root: &Path,
    workspace_path: Option<&Path>,
) -> Result<Vec<AppSessionSummary>> {
    match workspace_path {
        Some(workspace) => list_workspace(config_root, workspace, CatalogMode::Audit),
        None => list_all(config_root, CatalogMode::Audit),
    }
}

fn list_all(config_root: &Path, mode: CatalogMode) -> Result<Vec<AppSessionSummary>> {
    let sessions_root = config_root.join("sessions");
    let workspace_dirs = match std::fs::read_dir(&sessions_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", sessions_root.display()));
        }
    };

    let mut summaries = Vec::new();
    for workspace_entry in workspace_dirs {
        let workspace_entry = workspace_entry?;
        if !workspace_entry.file_type()?.is_dir() {
            continue;
        }

        for session_entry in std::fs::read_dir(workspace_entry.path())? {
            let session_entry = session_entry?;
            if !session_entry.file_type()?.is_dir() {
                continue;
            }

            let session_dir = session_entry.path();
            if let Some(summary) = catalog_summary(session_dir, mode)? {
                summaries.push(summary);
            }
        }
    }

    summaries.sort_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| right.session_dir.cmp(&left.session_dir))
    });
    Ok(summaries)
}

fn list_workspace(
    config_root: &Path,
    workspace_path: &Path,
    mode: CatalogMode,
) -> Result<Vec<AppSessionSummary>> {
    let workspace_dir = config_root
        .join("sessions")
        .join(encode_workspace_path(workspace_path)?);
    let session_dirs = match std::fs::read_dir(&workspace_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", workspace_dir.display()));
        }
    };

    let mut summaries = Vec::new();
    for session_entry in session_dirs {
        let session_entry = session_entry?;
        if !session_entry.file_type()?.is_dir() {
            continue;
        }
        if let Some(summary) = catalog_summary(session_entry.path(), mode)? {
            summaries.push(summary);
        }
    }
    summaries.sort_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| right.session_dir.cmp(&left.session_dir))
    });
    Ok(summaries)
}

fn catalog_summary(session_dir: PathBuf, mode: CatalogMode) -> Result<Option<AppSessionSummary>> {
    if matches!(mode, CatalogMode::Audit) {
        let summary = session_summary_from_dir(session_dir)?;
        return Ok((summary.message_count > 0).then_some(summary));
    }
    // The catalog isolates unusable entries, never accepts or rewrites them.
    match session_summary_from_dir(session_dir.clone()) {
        Ok(summary) => Ok((summary.message_count > 0).then_some(summary)),
        Err(error) => {
            eprintln!(
                "warning: skipping unusable persisted session {}: {error:#}",
                session_dir.display()
            );
            Ok(None)
        }
    }
}

fn session_summary_from_dir(session_dir: PathBuf) -> Result<AppSessionSummary> {
    let session_id = resolve_session_identity(&session_dir)?.session_id;
    let workspace_path = workspace_path_from_session_dir(&session_dir)?;
    let projection = JournalProjection::build(session_id, load_records(&session_dir, session_id)?)?;
    let (message_count, preview) = messages_summary(&projection.history);
    let updated_at_ms = session_updated_at_ms(&journal_path(&session_dir));

    Ok(AppSessionSummary::new(
        session_dir,
        session_id,
        workspace_path,
        message_count,
        updated_at_ms,
        preview,
    ))
}

fn messages_summary(messages: &[CanonicalMessage]) -> (usize, Option<String>) {
    let mut first_text_preview = None;
    let mut first_user_preview = None;
    for message in messages {
        if let Some(text) = message_text_preview(message) {
            if first_text_preview.is_none() {
                first_text_preview = Some(text.clone());
            }
            if message.role == MessageRole::User && first_user_preview.is_none() {
                first_user_preview = Some(text);
            }
        }
    }
    (messages.len(), first_user_preview.or(first_text_preview))
}

fn message_text_preview(message: &CanonicalMessage) -> Option<String> {
    message.parts.iter().find_map(|part| match &part.payload {
        ContentPart::Text { text }
        | ContentPart::ReasoningSummary { text }
        | ContentPart::Reasoning { text, signature: _ } => {
            let text = text.trim();
            (!text.is_empty()).then(|| truncate_preview(text))
        }
        ContentPart::ToolResult { result } => {
            let text = result.text_or_status();
            let text = text.trim();
            (!text.is_empty()).then(|| truncate_preview(text))
        }
        _ => None,
    })
}

fn truncate_preview(text: &str) -> String {
    let limit = 160;
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        format!("{}...", text.chars().take(limit).collect::<String>())
    }
}

fn session_updated_at_ms(path: &Path) -> Option<u64> {
    path.metadata()
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests;
