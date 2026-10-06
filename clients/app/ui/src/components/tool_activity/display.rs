use serde_json::Value;
use std::sync::Arc;

use super::headline::{ToolHeadline, tool_headline};
use crate::tool_names::{APPLY_PATCH_TOOL, UPDATE_PLAN_TOOL};
use crate::types::ToolActivity;
use crate::ui_utils::{compact_text, format_json};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ToolDisplay {
    pub(super) headline: ToolHeadline,
    pub(super) args: Vec<ToolArgPreview>,
    pub(super) patch_files: Vec<PatchFilePreview>,
    pub(super) plan_steps: Vec<PlanStepPreview>,
}

/// The arguments of an invocation cannot change during status or result updates.
/// Keep the parsed display and the source used to validate that assumption together.
pub(super) struct ToolStatic {
    call_id: String,
    name: String,
    args: Value,
    args_preview: String,
    effective_args: Option<Value>,
    pub(super) display: ToolDisplay,
    pub(super) args_text: String,
    pub(super) requested_json: String,
    pub(super) effective_json: String,
}

impl ToolStatic {
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    fn matches(&self, tool: &ToolActivity) -> bool {
        self.call_id == tool.call_id
            && self.name == tool.name
            && self.effective_args == tool.effective_args
            && self.args == tool.args
            && self.args_preview == tool.args_preview
    }

    fn new(tool: &ToolActivity) -> Self {
        Self {
            call_id: tool.call_id.clone(),
            name: tool.name.clone(),
            effective_args: tool.effective_args.clone(),
            args: tool.args.clone(),
            args_preview: tool.args_preview.clone(),
            display: tool_display(tool),
            args_text: tool_activity_args_preview(tool),
            requested_json: format_json(&tool.args),
            effective_json: tool
                .effective_args
                .as_ref()
                .filter(|args| **args != tool.args)
                .map(format_json)
                .unwrap_or_default(),
        }
    }
}

pub(super) fn tool_static_projection(
    previous: Option<&Option<Arc<ToolStatic>>>,
    tool: Option<&ToolActivity>,
) -> Option<Arc<ToolStatic>> {
    let tool = tool?;
    if let Some(previous) = previous.and_then(Option::as_ref)
        && previous.matches(tool)
    {
        return Some(previous.clone());
    }
    Some(Arc::new(ToolStatic::new(tool)))
}

pub(super) fn tool_static_changed(
    previous: Option<&Option<Arc<ToolStatic>>>,
    next: Option<&Option<Arc<ToolStatic>>>,
) -> bool {
    match (
        previous.and_then(Option::as_ref),
        next.and_then(Option::as_ref),
    ) {
        (None, None) => false,
        (Some(previous), Some(next)) => !Arc::ptr_eq(previous, next),
        _ => true,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlanStepPreview {
    pub(crate) step: String,
    pub(crate) status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ToolArgPreview {
    pub(super) key: String,
    pub(super) value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PatchFilePreview {
    pub(super) path: String,
    pub(super) operation: PatchOperation,
    pub(super) additions: usize,
    pub(super) deletions: usize,
    pub(super) body: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PatchOperation {
    Add,
    Delete,
    Update,
    Move,
}

impl PatchOperation {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Add => "создан",
            Self::Delete => "удалён",
            Self::Update => "изменён",
            Self::Move => "перемещён",
        }
    }

    /// Класс для цветовой метки операции в строке файла.
    pub(super) fn class(self) -> &'static str {
        match self {
            Self::Add => "tool-file-op op-add",
            Self::Delete => "tool-file-op op-delete",
            Self::Update => "tool-file-op op-update",
            Self::Move => "tool-file-op op-move",
        }
    }
}

/// Headline without the argument list, for compact rows outside the card.
pub(crate) fn tool_activity_headline(tool: &ToolActivity) -> ToolHeadline {
    tool_display(tool).headline
}

pub(super) fn tool_display(tool: &ToolActivity) -> ToolDisplay {
    let patch = if tool.name == APPLY_PATCH_TOOL {
        apply_patch_text_from_args(tool.invocation_args())
            .or_else(|| apply_patch_text_from_args_preview(&tool.args_preview))
    } else {
        None
    };
    let patch_files = patch
        .as_deref()
        .map(parse_apply_patch_files)
        .unwrap_or_default();
    let plan_steps = if tool.name == UPDATE_PLAN_TOOL {
        parse_plan_steps(tool.invocation_args())
    } else {
        Vec::new()
    };
    let mut headline = tool_headline(&tool.name, tool.invocation_args());
    if !patch_files.is_empty() {
        headline.subject = Some(patch_subject(&patch_files));
        headline.meta = Some(patch_stats(&patch_files));
    } else if !plan_steps.is_empty() {
        headline.subject = Some(plan_summary(&plan_steps));
    }
    let args = if patch_files.is_empty() && plan_steps.is_empty() {
        tool_arg_previews(tool.invocation_args(), headline.subject_key.as_deref())
    } else {
        Vec::new()
    };

    ToolDisplay {
        headline,
        args,
        patch_files,
        plan_steps,
    }
}

pub(crate) fn parse_plan_steps(args: &Value) -> Vec<PlanStepPreview> {
    args.get("plan")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|entry| {
                    let step = entry.get("step").and_then(Value::as_str)?;
                    let status = entry.get("status").and_then(Value::as_str)?;
                    Some(PlanStepPreview {
                        step: step.to_owned(),
                        status: status.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn plan_summary(steps: &[PlanStepPreview]) -> String {
    let completed = steps
        .iter()
        .filter(|step| step.status == "completed")
        .count();
    let current = steps
        .iter()
        .find(|step| step.status == "in_progress")
        .map(|step| format!(" · {}", step.step))
        .unwrap_or_default();
    format!("{}/{}{}", completed, steps.len(), current)
}

pub(super) fn tool_activity_args_preview(tool: &ToolActivity) -> String {
    if tool.name == APPLY_PATCH_TOOL {
        apply_patch_text_from_args(tool.invocation_args())
            .or_else(|| apply_patch_text_from_args_preview(&tool.args_preview))
            .unwrap_or_else(|| tool.args_preview.clone())
    } else {
        tool.args_preview.clone()
    }
}

pub(crate) fn tool_args_preview(tool_name: &str, args: &Value) -> String {
    if tool_name == APPLY_PATCH_TOOL {
        apply_patch_text_from_args(args).unwrap_or_else(|| format_json(args))
    } else {
        format_json(args)
    }
}

fn apply_patch_text_from_args_preview(args_preview: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(args_preview).ok()?;
    apply_patch_text_from_args(&value)
}

fn apply_patch_text_from_args(args: &Value) -> Option<String> {
    args.get("patch")
        .and_then(Value::as_str)
        .or_else(|| args.get("input").and_then(Value::as_str))
        .filter(|patch| !patch.trim().is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn parse_apply_patch_files(patch: &str) -> Vec<PatchFilePreview> {
    let mut files = Vec::new();
    let mut current: Option<PatchFilePreviewBuilder> = None;

    for line in patch.lines() {
        if line == "*** Begin Patch" || line == "*** End Patch" {
            continue;
        }

        if let Some((operation, path)) = apply_patch_file_header(line) {
            if let Some(builder) = current.take() {
                files.push(builder.finish());
            }
            current = Some(PatchFilePreviewBuilder::new(operation, path, line));
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Move to: ") {
            if let Some(builder) = current.as_mut() {
                builder.operation = PatchOperation::Move;
                builder.path = format!("{} -> {path}", builder.path);
                builder.push(line);
            }
            continue;
        }

        if let Some(builder) = current.as_mut() {
            builder.push(line);
        }
    }

    if let Some(builder) = current {
        files.push(builder.finish());
    }

    files
}

fn apply_patch_file_header(line: &str) -> Option<(PatchOperation, String)> {
    [
        ("*** Add File: ", PatchOperation::Add),
        ("*** Delete File: ", PatchOperation::Delete),
        ("*** Update File: ", PatchOperation::Update),
    ]
    .into_iter()
    .find_map(|(prefix, operation)| {
        line.strip_prefix(prefix)
            .map(|path| (operation, path.to_owned()))
    })
}

struct PatchFilePreviewBuilder {
    path: String,
    operation: PatchOperation,
    additions: usize,
    deletions: usize,
    body: Vec<String>,
}

impl PatchFilePreviewBuilder {
    fn new(operation: PatchOperation, path: String, header: &str) -> Self {
        Self {
            path,
            operation,
            additions: 0,
            deletions: 0,
            body: vec![header.to_owned()],
        }
    }

    fn push(&mut self, line: &str) {
        if line.starts_with('+') {
            self.additions += 1;
        } else if line.starts_with('-') {
            self.deletions += 1;
        }
        self.body.push(line.to_owned());
    }

    fn finish(self) -> PatchFilePreview {
        PatchFilePreview {
            path: self.path,
            operation: self.operation,
            additions: self.additions,
            deletions: self.deletions,
            body: self.body.join("\n"),
        }
    }
}

fn patch_subject(files: &[PatchFilePreview]) -> String {
    match files {
        [only] => only.path.clone(),
        [first, rest @ ..] => format!("{} +{}", first.path, rest.len()),
        [] => String::new(),
    }
}

fn patch_stats(files: &[PatchFilePreview]) -> String {
    let additions = files.iter().map(|file| file.additions).sum::<usize>();
    let deletions = files.iter().map(|file| file.deletions).sum::<usize>();
    format!("+{additions} −{deletions}")
}

fn tool_arg_previews(args: &Value, subject_key: Option<&str>) -> Vec<ToolArgPreview> {
    let Some(map) = args.as_object() else {
        return Vec::new();
    };

    map.iter()
        .filter(|(key, value)| !value.is_null() && Some(key.as_str()) != subject_key)
        .take(6)
        .map(|(key, value)| ToolArgPreview {
            key: key.clone(),
            value: tool_arg_value_preview(value),
        })
        .collect()
}

fn tool_arg_value_preview(value: &Value) -> String {
    match value {
        Value::String(value) => compact_text(value, 160),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(items) => {
            if items.is_empty() {
                "[]".to_owned()
            } else {
                format!("[{}]", item_count_label(items.len()))
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                "{}".to_owned()
            } else {
                format!("{{{}}}", item_count_label(map.len()))
            }
        }
        Value::Null => "null".to_owned(),
    }
}

fn item_count_label(count: usize) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolActivityStatus;

    #[test]
    fn status_and_result_updates_reuse_parsed_arguments() {
        let args = serde_json::json!({
            "patch": "*** Begin Patch\n*** Add File: a.txt\n+hi\n*** End Patch"
        });
        let mut tool = ToolActivity {
            effective_args: None,
            call_id: "call-1".to_owned(),
            name: APPLY_PATCH_TOOL.to_owned(),
            args: args.clone(),
            args_preview: format_json(&args),
            started_at_ms: 10,
            finished_at_ms: None,
            status: ToolActivityStatus::Running,
            result_preview: None,
        };
        let first = tool_static_projection(None, Some(&tool));
        assert_eq!(
            first.as_ref().unwrap().display.headline.meta.as_deref(),
            Some("+1 −0")
        );

        tool.status = ToolActivityStatus::Done;
        tool.result_preview = Some("done".to_owned());
        let second = tool_static_projection(Some(&first), Some(&tool));
        assert!(!tool_static_changed(Some(&first), Some(&second)));

        tool.args = serde_json::json!({
            "patch": "*** Begin Patch\n*** Add File: b.txt\n+new\n+line\n*** End Patch"
        });
        tool.args_preview = format_json(&tool.args);
        let third = tool_static_projection(Some(&second), Some(&tool));
        assert!(tool_static_changed(Some(&second), Some(&third)));
        assert_eq!(
            third.as_ref().unwrap().display.headline.text(),
            "Правка b.txt"
        );
        assert_eq!(
            third.as_ref().unwrap().display.headline.meta.as_deref(),
            Some("+2 −0")
        );
    }

    #[test]
    fn apply_patch_args_preview_extracts_patch_body() {
        let patch = "*** Begin Patch\n*** Add File: a.txt\n+hi\n*** End Patch";
        let args = serde_json::json!({ "patch": patch });

        assert_eq!(tool_args_preview("apply_patch", &args), patch);
        assert!(tool_args_preview("shell", &args).contains("\"patch\""));
    }

    #[test]
    fn apply_patch_args_preview_extracts_freeform_input() {
        let patch = "*** Begin Patch\n*** Update File: a.txt\n-old\n+new\n*** End Patch";
        let args = serde_json::json!({ "input": patch });

        assert_eq!(tool_args_preview("apply_patch", &args), patch);
    }

    #[test]
    fn apply_patch_display_groups_files_with_line_stats() {
        let patch = "\
*** Begin Patch
*** Add File: a.txt
+one
+two
*** Update File: src/lib.rs
@@
-old
+new
 context
*** End Patch";
        let files = parse_apply_patch_files(patch);

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[0].operation, PatchOperation::Add);
        assert_eq!(files[0].additions, 2);
        assert_eq!(files[0].deletions, 0);
        assert_eq!(files[1].path, "src/lib.rs");
        assert_eq!(files[1].operation, PatchOperation::Update);
        assert_eq!(files[1].additions, 1);
        assert_eq!(files[1].deletions, 1);
    }

    #[test]
    fn tool_display_summarizes_apply_patch_instead_of_raw_args() {
        let patch = "*** Begin Patch\n*** Add File: a.txt\n+hi\n*** End Patch";
        let args = serde_json::json!({ "patch": patch });
        let display = tool_display(&ToolActivity {
            effective_args: None,
            call_id: "call-1".to_owned(),
            name: "apply_patch".to_owned(),
            args: args.clone(),
            args_preview: format_json(&args),
            started_at_ms: 0,
            finished_at_ms: None,
            status: ToolActivityStatus::Done,
            result_preview: None,
        });

        assert_eq!(display.headline.text(), "Правка a.txt");
        assert_eq!(display.headline.meta.as_deref(), Some("+1 −0"));
        assert!(display.args.is_empty());
        assert_eq!(display.patch_files.len(), 1);
    }

    #[test]
    fn tool_display_summarizes_generic_args() {
        let args = serde_json::json!({
            "path": "src/lib.rs",
            "limit": 20,
            "hidden": null
        });
        let display = tool_display(&ToolActivity {
            effective_args: None,
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            args: args.clone(),
            args_preview: format_json(&args),
            started_at_ms: 0,
            finished_at_ms: None,
            status: ToolActivityStatus::Done,
            result_preview: None,
        });

        assert_eq!(display.headline.text(), "Чтение src/lib.rs");
        assert_eq!(
            display.args,
            vec![ToolArgPreview {
                key: "limit".to_owned(),
                value: "20".to_owned()
            }]
        );
    }
}
