use anyhow::{Result, bail};
use async_trait::async_trait;
use proteus_contracts::{
    contracts::{AgentToolRecorder, ExecutionAttribution, ExecutionRecorder},
    domain::{
        ExchangeId, ExecutionId, SessionId, ThreadId, ToolCall, ToolCallResolution, ToolResult,
        TurnId,
    },
    model_standard::{CanonicalModelRequest, CanonicalModelResponse},
};

use crate::core::SessionStore;

use super::{
    JournalEntry, ModelRequestRecorded, ModelResponseOutcome, ModelResponseRecorded,
    ToolCallRecordPhase, ToolCallRecorded, ToolResultRecorded,
};

#[derive(Debug, Clone)]
pub struct SessionExecutionRecorder {
    store: SessionStore,
    attribution: ExecutionAttribution,
}

impl SessionExecutionRecorder {
    pub fn for_turn(
        store: SessionStore,
        execution_id: ExecutionId,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> Self {
        let attribution =
            ExecutionAttribution::for_turn(execution_id, store.session_id(), thread_id, turn_id);
        Self { store, attribution }
    }

    pub fn detached(store: SessionStore, execution_id: ExecutionId) -> Self {
        Self {
            store,
            attribution: ExecutionAttribution::detached(execution_id),
        }
    }

    pub fn execution_id(&self) -> ExecutionId {
        self.attribution.execution_id
    }
}

#[async_trait]
impl ExecutionRecorder for SessionExecutionRecorder {
    async fn model_request_recorded(
        &self,
        exchange_id: ExchangeId,
        request: &CanonicalModelRequest,
    ) -> Result<()> {
        self.store
            .append_execution_journal_entry(
                self.attribution,
                JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                    exchange_id,
                    request: request.clone(),
                }),
            )
            .await?;
        Ok(())
    }

    async fn model_response_recorded(
        &self,
        exchange_id: ExchangeId,
        response: &CanonicalModelResponse,
    ) -> Result<()> {
        self.record_model_outcome(
            exchange_id,
            ModelResponseOutcome::Response {
                response: response.clone(),
            },
        )
        .await
    }

    async fn model_error_recorded(&self, exchange_id: ExchangeId, message: &str) -> Result<()> {
        self.record_model_outcome(
            exchange_id,
            ModelResponseOutcome::Error {
                message: message.to_owned(),
            },
        )
        .await
    }
}

impl SessionExecutionRecorder {
    async fn record_model_outcome(
        &self,
        exchange_id: ExchangeId,
        outcome: ModelResponseOutcome,
    ) -> Result<()> {
        self.store
            .append_execution_journal_entry(
                self.attribution,
                JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                    exchange_id,
                    outcome,
                }),
            )
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SessionAgentToolRecorder {
    store: SessionStore,
    execution_id: ExecutionId,
}

impl SessionAgentToolRecorder {
    pub fn new(store: SessionStore, execution_id: ExecutionId) -> Self {
        Self {
            store,
            execution_id,
        }
    }

    pub fn execution_id(&self) -> ExecutionId {
        self.execution_id
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

    fn attribution(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> Result<ExecutionAttribution> {
        self.validate_session(session_id)?;
        Ok(ExecutionAttribution::for_turn(
            self.execution_id,
            session_id,
            thread_id,
            turn_id,
        ))
    }
}

#[async_trait]
impl AgentToolRecorder for SessionAgentToolRecorder {
    async fn tool_call_requested(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        call: &ToolCall,
    ) -> Result<()> {
        let attribution = self.attribution(session_id, thread_id, turn_id)?;
        self.store
            .append_execution_journal_entry(
                attribution,
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
        let attribution = self.attribution(session_id, thread_id, turn_id)?;
        self.store
            .append_execution_journal_entry(
                attribution,
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

    async fn tool_approval_requested(
        &self,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        call: &ToolCall,
        reason: &str,
    ) -> Result<()> {
        let attribution = self.attribution(session_id, thread_id, turn_id)?;
        self.store
            .append_execution_journal_entry(
                attribution,
                JournalEntry::ToolCallRecorded(ToolCallRecorded {
                    call: call.clone(),
                    phase: ToolCallRecordPhase::ApprovalRequested {
                        reason: reason.to_owned(),
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
        let attribution = self.attribution(session_id, thread_id, turn_id)?;
        self.store
            .append_execution_journal_entry(
                attribution,
                JournalEntry::ToolResultRecorded(ToolResultRecorded {
                    result: result.clone(),
                }),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use proteus_contracts::domain::{
        AgentTask, ToolCall, new_call_id, new_execution_id, new_session_id, new_thread_id,
        new_turn_id,
    };
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn session_recorders_share_execution_owner_and_tools_keep_dynamic_threads() {
        let config_dir = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let session_id = new_session_id();
        let execution_id = new_execution_id();
        let root_thread_id = new_thread_id();
        let child_thread_id = new_thread_id();
        let turn_id = new_turn_id();
        let store = SessionStore::new(config_dir.path(), workspace.path(), session_id).unwrap();
        store
            .append_execution_journal_entry(
                ExecutionAttribution::for_turn(execution_id, session_id, root_thread_id, turn_id),
                JournalEntry::TurnOpened(crate::core::TurnOpened {
                    task: AgentTask::new("record tools", workspace.path().to_path_buf()),
                    base_history_revision: 0,
                    module_epoch: 0,
                    config_snapshot: json!({}),
                }),
            )
            .await
            .unwrap();
        let model_recorder = SessionExecutionRecorder::for_turn(
            store.clone(),
            execution_id,
            root_thread_id,
            turn_id,
        );
        let recorder = SessionAgentToolRecorder::new(store.clone(), execution_id);

        recorder
            .tool_call_requested(
                session_id,
                root_thread_id,
                turn_id,
                &ToolCall::new(new_call_id(), "root", json!({})),
            )
            .await
            .unwrap();
        recorder
            .tool_call_requested(
                session_id,
                child_thread_id,
                turn_id,
                &ToolCall::new(new_call_id(), "child", json!({})),
            )
            .await
            .unwrap();

        assert_eq!(model_recorder.execution_id(), execution_id);
        assert_eq!(recorder.execution_id(), execution_id);
        assert_eq!(
            store
                .load_records()
                .unwrap()
                .into_iter()
                .filter(|record| matches!(record.entry, JournalEntry::ToolCallRecorded(_)))
                .map(|record| record.thread_id)
                .collect::<Vec<_>>(),
            vec![Some(root_thread_id), Some(child_thread_id)]
        );
    }

    #[tokio::test]
    async fn detached_execution_recorder_persists_without_chat_identity() {
        let config_dir = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let session_id = new_session_id();
        let execution_id = new_execution_id();
        let store = SessionStore::new(config_dir.path(), workspace.path(), session_id).unwrap();
        let recorder = SessionExecutionRecorder::detached(store.clone(), execution_id);
        let exchange_id = crate::domain::new_exchange_id();
        let request = crate::model_standard::CanonicalModelRequest::new(
            crate::domain::ModelRef::new("fake", "detached"),
            vec![crate::model_standard::CanonicalMessage::text(
                crate::model_standard::MessageRole::User,
                "detached",
            )],
        );

        recorder
            .model_request_recorded(exchange_id, &request)
            .await
            .unwrap();
        recorder
            .model_error_recorded(exchange_id, "expected test error")
            .await
            .unwrap();

        let records = store.load_projection().unwrap().records;
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| {
            record.execution_id == Some(execution_id)
                && record.thread_id.is_none()
                && record.turn_id.is_none()
        }));
    }
}
