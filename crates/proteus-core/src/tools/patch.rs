use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{PatchApplier, Tool, ToolContext},
    domain::{Patch, ToolCall, ToolResult, ToolSafety, ToolSpec},
};

#[derive(Clone)]
pub struct ApplyPatchTool {
    patch: Arc<dyn PatchApplier>,
}

impl ApplyPatchTool {
    pub fn new(patch: Arc<dyn PatchApplier>) -> Self {
        Self { patch }
    }
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "apply_patch",
            "Apply a workspace patch through the configured PatchApplier. Use the syntax documented by the selected patch module in the profile instructions.",
            json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "Patch text in the configured patch module's format."
                    }
                },
                "required": ["patch"]
            }),
            ToolSafety::WritesFiles,
        )
        .with_timeout(10_000)
        .with_metadata(json!({
            "hot": true,
            "category": "patch",
            "tags": ["workspace", "edit", "patch", "write"],
            "aliases": ["apply changes", "edit files", "modify workspace"],
            "approval": {
                "cache_scopes": ["workspace_write"]
            }
        }))
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> Result<ToolResult> {
        let patch = patch_text_from_call(call)?;
        let result = self.patch.apply(Patch::new(patch)).await?;
        Ok(ToolResult::new(
            call.id.clone(),
            result.ok,
            result.summary,
            Vec::new(),
            None,
            json!({}),
        ))
    }
}

fn patch_text_from_call(call: &ToolCall) -> Result<&str> {
    if matches!(call.surface, crate::domain::ToolCallSurface::Freeform) {
        return call
            .args
            .get("input")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("apply_patch requires string arg 'input'"));
    }

    call.args
        .get("patch")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("apply_patch requires string arg 'patch'"))
}
