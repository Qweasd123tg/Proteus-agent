use super::AgentRuntime;
use crate::{
    contracts::{CancellationToken, ToolUserCommand},
    core::bound_tools::NoopToolExecutionObserver,
    domain::{ToolCall, ToolResult, new_call_id},
};
use anyhow::{Result, anyhow};
use std::sync::Arc;

impl AgentRuntime {
    pub async fn user_commands(&self) -> Vec<(ToolUserCommand, String)> {
        self.capture_execution_snapshot()
            .await
            .runtime
            .registry
            .tools
            .user_commands()
    }

    /// Explicit user operation: no model/Turn required, normal tool safety applies.
    pub async fn execute_user_command(
        &self,
        name: &str,
        arguments: &str,
        cancellation: CancellationToken,
    ) -> Result<ToolResult> {
        let admission = self.admit_execution(cancellation).await;
        let (_, tool) = admission
            .snapshot
            .runtime
            .registry
            .tools
            .user_commands()
            .into_iter()
            .find(|(command, _)| command.name == name)
            .ok_or_else(|| anyhow!("unknown module command: /{name}"))?;
        let conversation = Arc::new(super::conversation::TurnConversation {
            history: self.session.history.clone(),
            context: self.session.model_context.clone(),
        });
        self.bind_detached_tools(&admission)
            .execute_enriched(
                self.services.cwd.clone(),
                ToolCall::new(
                    new_call_id(),
                    tool,
                    serde_json::json!({"arguments": arguments}),
                ),
                &NoopToolExecutionObserver,
                |ctx| {
                    ctx.conversation = Some(conversation);
                    ctx.conversation_session_id = Some(self.session.session_id);
                },
            )
            .await
    }
}
