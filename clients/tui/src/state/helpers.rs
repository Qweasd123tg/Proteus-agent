use std::{path::Path, time::Duration};

use agent_contracts::domain::{SessionId, ToolResult};

pub(super) fn format_duration_short(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

pub(super) fn preview(result: &ToolResult) -> String {
    if let Some(error) = &result.error {
        return error.clone();
    }

    let mut out = String::new();
    for ch in result.output.chars() {
        match ch {
            '\t' => out.push_str("  "),
            '\r' => {}
            other => out.push(other),
        }
        if out.chars().count() >= 160 {
            break;
        }
    }
    out
}

pub(super) fn footer_hint() -> String {
    "enter send · ctrl+c clear/quit".to_owned()
}

pub(super) fn session_label_from_dir(session_dir: &Path) -> String {
    session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(short_session_label)
        .unwrap_or_else(|| "persisted".to_owned())
}

pub(super) fn short_session_id(session_id: SessionId) -> String {
    short_session_label(&session_id.to_string())
}

fn short_session_label(label: &str) -> String {
    let mut chars = label.chars();
    let short = chars.by_ref().take(10).collect::<String>();
    if chars.next().is_some() {
        format!("{short}...")
    } else {
        short
    }
}

pub(super) fn is_large_paste(text: &str) -> bool {
    let char_count = text.chars().count();
    let line_count = text.lines().count().max(1);
    char_count > 1200 || line_count > 6
}
