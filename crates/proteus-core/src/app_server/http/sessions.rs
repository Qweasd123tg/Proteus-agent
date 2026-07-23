use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};

use crate::core::{SessionStore, canonicalize_session_dir_path};

use super::{HttpAppState, state::session_key as canonical_session_key};
use crate::app_server::{
    AppContextMapSnapshot, AppServerHandle, AppSessionActivity, AppSessionSummary,
    AppTranscriptMessage, journal_transcript_messages,
};

pub(super) async fn session_summaries(
    state: &HttpAppState,
    current_workspace_only: bool,
) -> Result<Vec<AppSessionSummary>> {
    let current = state.current_server().await;
    let stored_summaries = if current_workspace_only {
        current.workspace_session_summaries()?
    } else {
        current.session_summaries()?
    };
    let activity_by_dir = state.activity_by_session_dir().await;
    let mut seen = HashSet::new();
    let mut summaries = Vec::new();
    let current_session_key = current.session_dir_path().map(canonical_session_key);

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
        if current_workspace_only
            && !super::super::paths_equal(server.cwd_path(), current.cwd_path())
        {
            continue;
        }
        let activity = state.activity_for_server(&server).await;
        let include_empty_idle = Some(&session_key) == current_session_key.as_ref();
        if let Some(summary) =
            known_session_summary(&server, &session_dir, activity, include_empty_idle).await?
        {
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
    include_empty_idle: bool,
) -> Result<Option<AppSessionSummary>> {
    let transcript = server.transcript().await?;
    let message_count = transcript.len();
    if message_count == 0 && activity.is_idle() && !include_empty_idle {
        return Ok(None);
    }

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
    let Some(session_dir) = query_path_param(query, "session_dir")? else {
        return state.current_server().await.transcript().await;
    };
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
    let Some(session_dir) = query_path_param(query, "session_dir")? else {
        let server = state.current_server().await;
        let activity = state.activity_for_server(&server).await;
        return server.context_map_snapshot(Some(activity)).await;
    };
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    if let Some(server) = state.server_for_session_dir(&session_dir).await {
        let activity = state.activity_for_server(&server).await;
        return server.context_map_snapshot(Some(activity)).await;
    }

    state
        .current_server()
        .await
        .context_map_snapshot_for_session_dir(session_dir, None)
        .await
}

pub(super) async fn server_for_optional_session(
    state: &HttpAppState,
    session_dir: Option<PathBuf>,
) -> Result<AppServerHandle> {
    let Some(session_dir) = session_dir else {
        return Ok(state.current_server().await);
    };
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

fn query_path_param(query: Option<&str>, name: &str) -> Result<Option<PathBuf>> {
    let Some(query) = query else {
        return Ok(None);
    };
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == name {
            return Ok(Some(PathBuf::from(percent_decode_query_value(value)?)));
        }
    }
    Ok(None)
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
