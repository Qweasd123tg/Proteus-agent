use anyhow::Result;
use async_trait::async_trait;

use crate::domain::{SessionId, ThreadId, ToolCall, ToolCallResolution, ToolResult, TurnId};

/// Core-owned execution facts exposed to the tool orchestration boundary.
/// Implementations may persist them, while the default keeps runtimes without
/// a session store fully in-memory.
#[async_trait]
pub trait ExecutionRecorder: Send + Sync {
    async fn tool_call_requested(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        call: &ToolCall,
    ) -> Result<()>;

    async fn tool_call_resolved(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        call: &ToolCall,
        resolution: &ToolCallResolution,
    ) -> Result<()>;

    async fn tool_result_recorded(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        result: &ToolResult,
    ) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NoopExecutionRecorder;

#[async_trait]
impl ExecutionRecorder for NoopExecutionRecorder {
    async fn tool_call_requested(
        &self,
        _session_id: SessionId,
        _thread_id: ThreadId,
        _turn_id: TurnId,
        _call: &ToolCall,
    ) -> Result<()> {
        Ok(())
    }

    async fn tool_call_resolved(
        &self,
        _session_id: SessionId,
        _thread_id: ThreadId,
        _turn_id: TurnId,
        _call: &ToolCall,
        _resolution: &ToolCallResolution,
    ) -> Result<()> {
        Ok(())
    }

    async fn tool_result_recorded(
        &self,
        _session_id: SessionId,
        _thread_id: ThreadId,
        _turn_id: TurnId,
        _result: &ToolResult,
    ) -> Result<()> {
        Ok(())
    }
}
