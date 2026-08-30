use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::ExecutionAttribution,
    domain::{ToolCall, ToolCallResolution, ToolResult},
};

/// Recording surface for tool lifecycle facts owned by a logical execution.
///
/// Agent callers may attach their current chat projection to attribution;
/// detached execution never invents Session/Thread/Turn identities.
#[async_trait]
pub trait ToolExecutionRecorder: Send + Sync {
    async fn tool_call_requested(
        &self,
        attribution: ExecutionAttribution,
        call: &ToolCall,
    ) -> Result<()>;

    async fn tool_call_resolved(
        &self,
        attribution: ExecutionAttribution,
        call: &ToolCall,
        resolution: &ToolCallResolution,
    ) -> Result<()>;

    async fn tool_approval_requested(
        &self,
        attribution: ExecutionAttribution,
        call: &ToolCall,
        reason: &str,
    ) -> Result<()>;

    async fn tool_result_recorded(
        &self,
        attribution: ExecutionAttribution,
        result: &ToolResult,
    ) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NoopToolExecutionRecorder;

#[async_trait]
impl ToolExecutionRecorder for NoopToolExecutionRecorder {
    async fn tool_call_requested(
        &self,
        _attribution: ExecutionAttribution,
        _call: &ToolCall,
    ) -> Result<()> {
        Ok(())
    }

    async fn tool_call_resolved(
        &self,
        _attribution: ExecutionAttribution,
        _call: &ToolCall,
        _resolution: &ToolCallResolution,
    ) -> Result<()> {
        Ok(())
    }

    async fn tool_approval_requested(
        &self,
        _attribution: ExecutionAttribution,
        _call: &ToolCall,
        _reason: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn tool_result_recorded(
        &self,
        _attribution: ExecutionAttribution,
        _result: &ToolResult,
    ) -> Result<()> {
        Ok(())
    }
}
