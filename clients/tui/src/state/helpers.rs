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
        let output = render_preview_body(result, false);
        if output.is_empty() {
            return error.clone();
        }
        return format!("{output}\n{error}");
    }

    render_preview_output(result)
}

fn render_preview_output(result: &ToolResult) -> String {
    render_preview_body(result, true)
}

fn render_preview_body(result: &ToolResult, status_on_empty: bool) -> String {
    let limit = if is_user_input_result(result) {
        2_000
    } else {
        160
    };
    let output = if !result.output.is_empty() {
        result.output.clone()
    } else if status_on_empty {
        result.text_or_status()
    } else {
        result_output_content(result)
    };
    let mut out = String::new();
    for ch in output.chars() {
        match ch {
            '\t' => out.push_str("  "),
            '\r' => {}
            other => out.push(other),
        }
        if out.chars().count() >= limit {
            break;
        }
    }
    out
}

fn result_output_content(result: &ToolResult) -> String {
    let mut content = Vec::new();
    for item in &result.content {
        match item {
            agent_contracts::domain::ToolContent::Text { text } if !text.is_empty() => {
                content.push(text.clone());
            }
            agent_contracts::domain::ToolContent::Json { value } => {
                content.push(value.to_string());
            }
            agent_contracts::domain::ToolContent::Image { mime_type, .. } => {
                content.push(format!("[image tool content: {mime_type}]"));
            }
            agent_contracts::domain::ToolContent::Binary { mime_type, .. } => {
                content.push(format!("[binary tool content: {mime_type}]"));
            }
            _ => {}
        }
    }
    content.join("\n")
}

pub(super) fn is_user_input_result(result: &ToolResult) -> bool {
    matches!(
        result.metadata.get("tool").and_then(|tool| tool.as_str()),
        Some("request_user_input" | "AskUserQuestion")
    ) || result.output.starts_with("User answered:\n")
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
