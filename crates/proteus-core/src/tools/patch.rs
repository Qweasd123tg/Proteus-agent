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
            "Apply a workspace patch through the configured PatchApplier. The default direct patcher uses Proteus internal patch format, not unified diff: do not send diff --git, ---/+++, @@ -line,count headers, or replace file:N-M commands.",
            json!({
                "type": "object",
                "description": "Patch request. For modules.patch = \"direct\", patch must use Proteus internal patch format with *** Begin Patch / *** End Patch and *** Add File, *** Update File, or *** Delete File operations. Unified diff is not accepted.",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "Internal patch text. Example update: *** Begin Patch\n*** Update File: src/main.rs\n@@\n-old line\n+new line\n*** End Patch"
                    }
                },
                "required": ["patch"]
            }),
            ToolSafety::WritesFiles,
        )
        .with_timeout(10_000)
        .with_metadata(json!({
            "format": "internal_patch",
            "accepted_headers": [
                "*** Add File: <path>",
                "*** Update File: <path>",
                "*** Delete File: <path>",
                "*** Move to: <path>"
            ],
            "unsupported_formats": [
                "diff --git",
                "--- a/file and +++ b/file unified diff headers",
                "@@ -line,count +line,count @@ unified diff hunks",
                "replace file:start-end"
            ],
            "example_add": "*** Begin Patch\n*** Add File: notes.txt\n+first line\n+second line\n*** End Patch",
            "example_update": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old line\n+new line\n*** End Patch",
            "example_delete": "*** Begin Patch\n*** Delete File: obsolete.txt\n*** End Patch"
        }))
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
