use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};

use crate::app_server::context_map::{ContextMapInput, build_context_map_snapshot};
use crate::core::{
    SessionStore, canonicalize_session_dir_path, config_store_root, event_log_path,
    list_session_summaries, list_workspace_session_summaries,
};

use super::{HttpAppState, state::session_key as canonical_session_key};
use crate::app_server::{
    AppContextMapSnapshot, AppServerHandle, AppSessionActivity, AppSessionSummary,
    AppTranscriptMessage, journal_transcript_messages,
};

pub(super) async fn session_summaries(
    state: &HttpAppState,
    workspace: Option<PathBuf>,
) -> Result<Vec<AppSessionSummary>> {
    let stored_summaries = match state.launch.config_path.as_deref() {
        Some(path) => match workspace.as_deref() {
            Some(cwd) => list_workspace_session_summaries(&config_store_root(path), cwd)?,
            None => list_session_summaries(&config_store_root(path))?,
        },
        None => Vec::new(),
    };
    let activity_by_dir = state.activity_by_session_dir().await;
    let mut seen = HashSet::new();
    let mut summaries = Vec::new();

    for mut summary in stored_summaries {
        let session_dir = summary.session_dir.clone();
        let session_key = canonical_session_key(session_dir.clone());
        seen.insert(session_key.clone());
        if let Some(activity) = activity_by_dir.get(&session_key) {
            summary = summary.with_activity(activity.clone());
        }
        summaries.push(summary);
    }

    for server in state.all_servers().await {
        let Some(session_dir) = server.session_dir_path() else {
            continue;
        };
        let session_key = canonical_session_key(session_dir.clone());
        if seen.contains(&session_key) {
            continue;
        }
        if workspace
            .as_deref()
            .is_some_and(|cwd| !super::super::paths_equal(server.cwd_path(), cwd))
        {
            continue;
        }
        let activity = state.activity_for_server(&server).await;
        if let Some(summary) = known_session_summary(&server, &session_dir, activity).await? {
            seen.insert(session_key);
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

async fn known_session_summary(
    server: &AppServerHandle,
    session_dir: &Path,
    activity: AppSessionActivity,
) -> Result<Option<AppSessionSummary>> {
    let transcript = server.transcript().await?;
    let message_count = transcript.len();
    Ok(Some(
        AppSessionSummary::new(
            session_dir.to_path_buf(),
            server.session_id(),
            server.cwd_path().to_path_buf(),
            message_count,
            Some(current_time_ms()),
            transcript_preview(&transcript),
        )
        .with_activity(activity),
    ))
}

fn transcript_preview(transcript: &[AppTranscriptMessage]) -> Option<String> {
    transcript
        .iter()
        .find(|message| message.role == "user" && !message.text.trim().is_empty())
        .or_else(|| {
            transcript
                .iter()
                .find(|message| !message.text.trim().is_empty())
        })
        .map(|message| truncate_session_preview(message.text.trim()))
}

fn truncate_session_preview(text: &str) -> String {
    let limit = 160;
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        format!("{}...", text.chars().take(limit).collect::<String>())
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

pub(super) async fn history_json(
    state: &HttpAppState,
    query: Option<&str>,
) -> Result<Vec<AppTranscriptMessage>> {
    let session_dir = required_session_query(query)?;
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    if let Some(server) = state.server_for_session_dir(&session_dir).await {
        return server.transcript().await;
    }

    let projection = SessionStore::open(session_dir)?.load_projection()?;
    Ok(journal_transcript_messages(&projection, None))
}

pub(super) async fn context_map_json(
    state: &HttpAppState,
    query: Option<&str>,
) -> Result<AppContextMapSnapshot> {
    let session_dir = required_session_query(query)?;
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    if let Some(server) = state.server_for_session_dir(&session_dir).await {
        let activity = state.activity_for_server(&server).await;
        return server.context_map_snapshot(Some(activity)).await;
    }

    let store = SessionStore::open(session_dir.clone())?;
    let workspace_path = store.workspace_path()?;
    build_context_map_snapshot(ContextMapInput {
        session_dir: Some(session_dir),
        session_id: Some(store.session_id()),
        event_log_path: event_log_path(
            &state.launch.config.event_log.path,
            state.launch.config_path.as_deref(),
            &workspace_path,
        ),
        workspace_path: Some(workspace_path),
        activity: None,
        history: store.load_messages()?,
        diagnostics: Vec::new(),
    })
}

pub(super) async fn usage_json(
    state: &HttpAppState,
    query: Option<&str>,
) -> Result<Option<crate::domain::SessionUsageSnapshot>> {
    let session_dir = required_session_query(query)?;
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    if let Some(server) = state.server_for_session_dir(&session_dir).await {
        return server.usage_snapshot().await;
    }
    SessionStore::open(session_dir)?
        .usage_snapshot()
        .await
        .map(Some)
}

pub(super) async fn server_for_session(
    state: &HttpAppState,
    session_dir: PathBuf,
) -> Result<AppServerHandle> {
    if !session_dir.is_absolute() {
        return Err(anyhow!("session_dir must be an absolute path"));
    }
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    state
        .server_for_session_dir(&session_dir)
        .await
        .ok_or_else(|| {
            anyhow!(
                "session is not active; resume it first: {}",
                session_dir.display()
            )
        })
}

pub(super) fn required_session_query(query: Option<&str>) -> Result<PathBuf> {
    let mut session_dir = None;
    for pair in query
        .unwrap_or_default()
        .split('&')
        .filter(|part| !part.is_empty())
    {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "session_dir" {
            if session_dir.is_some() {
                return Err(anyhow!("duplicate session_dir query parameter"));
            }
            let path = PathBuf::from(percent_decode_query_value(value)?);
            if !path.is_absolute() {
                return Err(anyhow!("session_dir must be an absolute path"));
            }
            session_dir = Some(path);
        }
    }
    session_dir.ok_or_else(|| anyhow!("missing required session_dir query parameter"))
}

pub(super) async fn server_for_query(
    state: &HttpAppState,
    query: Option<&str>,
) -> Result<AppServerHandle> {
    server_for_session(state, required_session_query(query)?).await
}

fn percent_decode_query_value(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                if let (Some(high), Some(low)) =
                    (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
                {
                    decoded.push((high << 4) | low);
                    index += 3;
                } else {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).map_err(anyhow::Error::from)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
