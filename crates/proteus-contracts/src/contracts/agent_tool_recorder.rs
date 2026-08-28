use anyhow::Result;
use async_trait::async_trait;

use crate::domain::{SessionId, ThreadId, ToolCall, ToolCallResolution, ToolResult, TurnId};

/// Agent/chat-owned recording surface for tool lifecycle facts.
///
/// Tool execution can move between presentation threads while remaining in
/// one logical execution, so the current chat projection remains explicit at
/// each call until its ownership is redesigned in a later phase.
#[async_trait]
pub trait AgentToolRecorder: Send + Sync {
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

    async fn tool_approval_requested(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        call: &ToolCall,
        reason: &str,
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
pub struct NoopAgentToolRecorder;

#[async_trait]
impl AgentToolRecorder for NoopAgentToolRecorder {
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

    async fn tool_approval_requested(
        &self,
        _session_id: SessionId,
        _thread_id: ThreadId,
        _turn_id: TurnId,
        _call: &ToolCall,
        _reason: &str,
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
