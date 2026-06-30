use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use serde_json::{Value, json};

use crate::domain::ToolCall;

use super::AppApprovalPreview;

const APPROVAL_PREVIEW_BODY_LIMIT: usize = 20_000;

pub(super) fn approval_preview_for(call: &ToolCall, cwd: &Path) -> Option<AppApprovalPreview> {
    match call.name.as_str() {
        "apply_patch" => approval_preview_for_apply_patch(call),
        "write_file" => approval_preview_for_write_file(call, cwd),
        "shell" => approval_preview_for_shell(call, cwd),
        _ => None,
    }
}

fn approval_preview_for_apply_patch(call: &ToolCall) -> Option<AppApprovalPreview> {
    let patch = call
        .args
        .get("patch")
        .and_then(Value::as_str)
        .or_else(|| call.args.get("input").and_then(Value::as_str))?;
    let affected_files = affected_files_from_internal_patch(patch);
    let summary = if affected_files.is_empty() {
        "Apply workspace patch".to_owned()
    } else if affected_files.len() == 1 {
        format!("Patch {}", affected_files[0])
    } else {
        format!("Patch {} files", affected_files.len())
    };

    Some(
        AppApprovalPreview::new("patch", "Patch preview", summary)
            .with_affected_files(affected_files)
            .with_body(truncate_preview_body(patch), "diff")
            .with_metadata(json!({ "format": "proteus_internal_patch" })),
    )
}

fn approval_preview_for_write_file(call: &ToolCall, cwd: &Path) -> Option<AppApprovalPreview> {
    let path = call.args.get("path").and_then(Value::as_str)?;
    let content = call.args.get("content").and_then(Value::as_str)?;
    let target = preview_target_path(cwd, path);
    let existing_content = target
        .as_ref()
        .and_then(|target| existing_preview_content(cwd, target));
    let operation = match (&target, &existing_content) {
        (_, Some(_)) => "overwrite",
        (Some(_), None) => "create",
        (None, None) => "write",
    };
    let summary = match operation {
        "overwrite" => format!("Overwrite {path} ({} bytes)", content.len()),
        "create" => format!("Create {path} ({} bytes)", content.len()),
        _ => format!("Write {path} ({} bytes)", content.len()),
    };
    let (body, language) = match existing_content {
        Some(existing) => (simple_line_diff(path, &existing, content), "diff"),
        None => (content.to_owned(), "text"),
    };

    Some(
        AppApprovalPreview::new("write_file", "File write preview", summary)
            .with_affected_files(vec![path.to_owned()])
            .with_body(truncate_preview_body(&body), language)
            .with_metadata(json!({
                "operation": operation,
                "path": path,
                "target": target.as_ref().map(|target| target.display().to_string()),
                "workspace_scoped": target.is_some(),
                "bytes": content.len(),
            })),
    )
}

fn approval_preview_for_shell(call: &ToolCall, cwd: &Path) -> Option<AppApprovalPreview> {
    let command = call.args.get("command").and_then(Value::as_str)?;
    Some(
        AppApprovalPreview::new(
            "command",
            "Command preview",
            format!("Run shell command in {}", cwd.display()),
        )
        .with_body(truncate_preview_body(command), "shell")
        .with_metadata(json!({
            "cwd": cwd.display().to_string(),
            "cache_scope": "exact_command",
        })),
    )
}

fn preview_target_path(cwd: &Path, path: &str) -> Option<PathBuf> {
    let base = std::fs::canonicalize(cwd).ok()?;
    let path = Path::new(path);
    let relative = if path.is_absolute() {
        path.strip_prefix(&base).ok()?
    } else {
        path
    };
    Some(base.join(safe_preview_relative_path(relative)?))
}

fn safe_preview_relative_path(path: &Path) -> Option<PathBuf> {
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if safe.as_os_str().is_empty() {
        None
    } else {
        Some(safe)
    }
}

fn existing_preview_content(cwd: &Path, target: &Path) -> Option<String> {
    let base = std::fs::canonicalize(cwd).ok()?;
    let metadata = std::fs::symlink_metadata(target).ok()?;
    if metadata.file_type().is_symlink() {
        return None;
    }
    let canonical_target = std::fs::canonicalize(target).ok()?;
    if !canonical_target.starts_with(base) {
        return None;
    }
    std::fs::read_to_string(canonical_target).ok()
}

fn affected_files_from_internal_patch(patch: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    for line in patch.lines() {
        for prefix in [
            "*** Add File:",
            "*** Update File:",
            "*** Delete File:",
            "*** Move to:",
        ] {
            if let Some(path) = line.strip_prefix(prefix) {
                let path = path.trim();
                if !path.is_empty() {
                    files.insert(path.to_owned());
                }
            }
        }
    }
    files.into_iter().collect()
}

fn simple_line_diff(path: &str, old: &str, new: &str) -> String {
    if old == new {
        return format!("No content change for {path}");
    }

    let old_lines = old.lines().collect::<Vec<_>>();
    let new_lines = new.lines().collect::<Vec<_>>();
    let mut diff = format!("--- {path}\n+++ {path}\n@@\n");
    for index in 0..old_lines.len().max(new_lines.len()) {
        match (old_lines.get(index), new_lines.get(index)) {
            (Some(old), Some(new)) if old == new => {
                diff.push(' ');
                diff.push_str(old);
                diff.push('\n');
            }
            (Some(old), Some(new)) => {
                diff.push('-');
                diff.push_str(old);
                diff.push('\n');
                diff.push('+');
                diff.push_str(new);
                diff.push('\n');
            }
            (Some(old), None) => {
                diff.push('-');
                diff.push_str(old);
                diff.push('\n');
            }
            (None, Some(new)) => {
                diff.push('+');
                diff.push_str(new);
                diff.push('\n');
            }
            (None, None) => {}
        }
    }
    diff
}

fn truncate_preview_body(body: &str) -> String {
    if body.len() <= APPROVAL_PREVIEW_BODY_LIMIT {
        return body.to_owned();
    }

    let end = body
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= APPROVAL_PREVIEW_BODY_LIMIT)
        .last()
        .unwrap_or(0);
    format!(
        "{}\n\n[approval preview truncated to {} bytes]",
        &body[..end],
        APPROVAL_PREVIEW_BODY_LIMIT
    )
}
