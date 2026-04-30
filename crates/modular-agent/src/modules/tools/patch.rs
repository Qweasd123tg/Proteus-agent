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
        ToolSpec {
            name: "apply_patch".to_owned(),
            description: "Apply a workspace-scoped patch using the internal patch format"
                .to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "patch": { "type": "string" }
                },
                "required": ["patch"]
            }),
            safety: ToolSafety::WritesFiles,
            timeout_ms: Some(10_000),
            metadata: json!({
                "format": "internal_patch",
                "example": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old line\n+new line\n*** End Patch"
            }),
        }
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> Result<ToolResult> {
        let patch = call
            .args
            .get("patch")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("apply_patch requires string arg 'patch'"))?;
        let result = self.patch.apply(Patch::new(patch)).await?;
        Ok(ToolResult::new(
            call.id.clone(),
            result.ok,
            result.summary,
            Vec::new(),
            None,
            json!({ "format": "internal_patch" }),
        ))
    }
}
