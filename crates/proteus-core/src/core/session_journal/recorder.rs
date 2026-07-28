use anyhow::{Result, bail};
use async_trait::async_trait;
use proteus_contracts::{
    contracts::ExecutionRecorder,
    domain::{SessionId, ThreadId, ToolCall, ToolCallResolution, ToolResult, TurnId},
};

use crate::core::SessionStore;

use super::{JournalEntry, ToolCallRecordPhase, ToolCallRecorded, ToolResultRecorded};

#[derive(Debug, Clone)]
pub struct SessionExecutionRecorder {
    store: SessionStore,
}

impl SessionExecutionRecorder {
    pub fn new(store: SessionStore) -> Self {
        Self { store }
    }

    fn validate_session(&self, session_id: SessionId) -> Result<()> {
        if self.store.session_id() != session_id {
            bail!(
                "execution recorder belongs to session {}, received {}",
                self.store.session_id(),
                session_id
            );
        }
        Ok(())
    }
}

#[async_trait]
impl ExecutionRecorder for SessionExecutionRecorder {
    async fn tool_call_requested(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        call: &ToolCall,
    ) -> Result<()> {
        self.validate_session(session_id)?;
        self.store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::ToolCallRecorded(ToolCallRecorded {
                    call: call.clone(),
                    phase: ToolCallRecordPhase::Requested,
                }),
            )
            .await?;
        Ok(())
    }

    async fn tool_call_resolved(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        call: &ToolCall,
        resolution: &ToolCallResolution,
    ) -> Result<()> {
        self.validate_session(session_id)?;
        self.store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::ToolCallRecorded(ToolCallRecorded {
                    call: call.clone(),
                    phase: ToolCallRecordPhase::Resolved {
                        resolution: resolution.clone(),
                    },
                }),
            )
            .await?;
        Ok(())
    }

    async fn tool_result_recorded(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        result: &ToolResult,
    ) -> Result<()> {
        self.validate_session(session_id)?;
        self.store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::ToolResultRecorded(ToolResultRecorded {
                    result: result.clone(),
                }),
            )
            .await?;
        Ok(())
    }
}
