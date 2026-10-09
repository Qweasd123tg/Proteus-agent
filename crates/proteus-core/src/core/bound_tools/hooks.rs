use super::{BoundTools, ToolExecutionObserver};
use crate::{
    contracts::{HookEvent, HookInput},
    domain::{ToolCall, ToolCallResolution, ToolResult, ToolSpec},
};
use anyhow::Result;
use std::path::Path;

impl BoundTools {
    fn hook_input(&self, cwd: &Path, event: HookEvent) -> HookInput {
        HookInput {
            conversation: None,
            event,
            attribution: self.binding.attribution,
            cwd: cwd.to_path_buf(),
        }
    }

    pub(super) async fn enforce_before_tool(
        &self,
        observer: &dyn ToolExecutionObserver,
        cwd: &Path,
        call: &mut ToolCall,
        spec: Option<ToolSpec>,
    ) -> Result<Option<ToolResult>> {
        let outcome = self
            .before_tool(cwd, call, spec)
            .await
            .map(|(effective, blocked)| {
                *call = effective;
                blocked
            });
        match outcome {
            Ok(Some(reason)) => {
                self.record_resolution(
                    call,
                    &ToolCallResolution::HookBlocked {
                        reason: reason.clone(),
                    },
                )
                .await?;
                Ok(Some(
                    self.finish(
                        observer,
                        cwd,
                        call,
                        ToolResult::error(call.id.clone(), reason),
                    )
                    .await?,
                ))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                let reason = error.to_string();
                self.record_resolution(
                    call,
                    &ToolCallResolution::HookBlocked {
                        reason: reason.clone(),
                    },
                )
                .await?;
                self.record_result(observer, ToolResult::error(call.id.clone(), reason))
                    .await?;
                Err(error)
            }
        }
    }

    pub(super) async fn before_tool(
        &self,
        cwd: &Path,
        call: &ToolCall,
        spec: Option<ToolSpec>,
    ) -> Result<(ToolCall, Option<String>)> {
        let event = self
            .hooks
            .apply(self.hook_input(
                cwd,
                HookEvent::BeforeTool {
                    call: call.clone(),
                    spec,
                    blocked: None,
                },
            ))
            .await?;
        let HookEvent::BeforeTool { call, blocked, .. } = event else {
            anyhow::bail!("hook changed tool event kind");
        };
        Ok((call, blocked))
    }

    pub(super) async fn finish(
        &self,
        observer: &dyn ToolExecutionObserver,
        cwd: &Path,
        call: &ToolCall,
        raw: ToolResult,
    ) -> Result<ToolResult> {
        let raw = self.truncate_result(raw);
        if self.hooks.is_active() {
            self.binding
                .recorder
                .tool_effect_recorded(self.binding.attribution, &raw)
                .await?;
        }
        let transformed = self
            .hooks
            .apply(self.hook_input(
                cwd,
                HookEvent::AfterTool {
                    call: call.clone(),
                    result: raw.clone(),
                },
            ))
            .await;
        match transformed {
            Ok(HookEvent::AfterTool { result, .. }) => {
                self.record_result(observer, self.truncate_result(result))
                    .await
            }
            Ok(_) => {
                self.record_result(observer, raw).await?;
                anyhow::bail!("hook changed tool result event kind");
            }
            Err(error) => {
                // The real tool has already settled. Preserve that fact even
                // when an output contribution fails; never replay its effect.
                self.record_result(observer, raw).await?;
                Err(error)
            }
        }
    }
}
