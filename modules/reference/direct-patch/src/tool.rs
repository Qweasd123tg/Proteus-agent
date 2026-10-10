use proteus_contracts::{
    domain::{ToolCall, ToolCallSurface, ToolResult, ToolSafety, ToolSpec},
    process_module::{
        ProcessModuleError, ToolModule, ToolModuleHostMut, ToolModuleInvocationContext,
    },
};
use serde_json::json;

pub(super) struct ApplyPatchTool;

impl ToolModule for ApplyPatchTool {
    fn spec_json(&self) -> String {
        let spec = ToolSpec::new(
            "apply_patch",
            "Apply a workspace patch using the direct line-based format documented in the profile instructions.",
            json!({
                "type": "object",
                "properties": {
                    "patch": {"type": "string", "description": "Patch text in the direct format."},
                    "workdir": {
                        "type": "string",
                        "description": "Existing working directory inside the workspace; relative paths are resolved from the workspace. Defaults to the workspace."
                    }
                },
                "required": ["patch"]
            }),
            ToolSafety::WritesFiles,
        ).with_timeout(10_000).with_metadata(json!({
            "hot": true,
            "category": "patch",
            "tags": ["workspace", "edit", "patch", "write"],
            "aliases": ["apply changes", "edit files", "modify workspace"],
            "approval": {"cache_scopes": ["workspace_write"]}
        }));
        serde_json::to_string(&spec).expect("serializable tool spec")
    }

    fn invoke_json(
        &self,
        call_json: String,
        context_json: String,
        _host: &mut ToolModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let call: ToolCall = serde_json::from_str(&call_json)
            .map_err(|error| ProcessModuleError::new(format!("invalid ToolCall JSON: {error}")))?;
        let context: ToolModuleInvocationContext = serde_json::from_str(&context_json)
            .map_err(|error| ProcessModuleError::new(format!("invalid tool context: {error}")))?;
        let key = if matches!(call.surface, ToolCallSurface::Freeform) {
            "input"
        } else {
            "patch"
        };
        let patch = call
            .args
            .get(key)
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                ProcessModuleError::new(format!("apply_patch requires string arg '{key}'"))
            })?;
        let workspace = context.cwd.canonicalize().map_err(|error| {
            ProcessModuleError::new(format!("resolve patch workspace: {error}"))
        })?;
        let workdir = match call.args.get("workdir") {
            Some(value) => value.as_str().ok_or_else(|| {
                ProcessModuleError::new("apply_patch requires string arg 'workdir'")
            })?,
            None => ".",
        };
        let cwd = workspace
            .join(workdir)
            .canonicalize()
            .map_err(|error| ProcessModuleError::new(format!("resolve patch workdir: {error}")))?;
        if !cwd.is_dir() {
            return Err(ProcessModuleError::new("patch workdir must be a directory"));
        }
        if !cwd.starts_with(&workspace) {
            return Err(ProcessModuleError::new("patch workdir escapes workspace"));
        }
        let summary = super::apply_patch(patch, &cwd).map_err(ProcessModuleError::new)?;
        serde_json::to_string(&ToolResult::new(
            call.id,
            true,
            summary,
            Vec::new(),
            None,
            json!({}),
        ))
        .map_err(|error| {
            ProcessModuleError::new(format!("failed to serialize ToolResult: {error}"))
        })
    }
}
