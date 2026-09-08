use anyhow::Result;

use crate::{
    contracts::WorkflowFailure,
    domain::{AgentOutput, TurnId},
    model_standard::CanonicalMessage,
};

use super::{
    AgentRuntime, prepare_failed_history_update,
    steering::{SteeringDeliveryRecord, weave_deliveries_into_failed_history},
};

impl AgentRuntime {
    pub(super) async fn fail_turn_with_progress(
        &self,
        turn_id: TurnId,
        turn_error: anyhow::Error,
        history: &[CanonicalMessage],
        current_user: &CanonicalMessage,
        deliveries: &[SteeringDeliveryRecord],
    ) -> Result<AgentOutput> {
        let progress = turn_error
            .downcast_ref::<WorkflowFailure>()
            .and_then(|failure| failure.history.clone());
        let Some(mut progress) = progress else {
            return self
                .fail_turn_preserving_steering(turn_id, turn_error, deliveries)
                .await;
        };

        let persist = async {
            let allowed = weave_deliveries_into_failed_history(&mut progress, deliveries)?;
            let update = prepare_failed_history_update(
                history,
                current_user,
                &progress.new_messages,
                progress.history_replacement.as_deref(),
                progress.compactions.iter().any(|report| report.changed),
                &allowed,
            )?;
            self.commit_history_update(
                turn_id,
                update,
                &progress.new_messages,
                &progress.compactions,
            )
            .await
        }
        .await;
        match persist {
            Ok(()) => Err(turn_error),
            Err(error) => {
                self.fail_turn_preserving_steering(
                    turn_id,
                    turn_error.context(format!("failed to persist workflow progress: {error:#}")),
                    deliveries,
                )
                .await
            }
        }
    }

    pub(super) async fn fail_turn_preserving_steering(
        &self,
        turn_id: TurnId,
        turn_error: anyhow::Error,
        deliveries: &[SteeringDeliveryRecord],
    ) -> Result<AgentOutput> {
        if let Err(persist_error) = self
            .persist_failed_steering_messages(turn_id, deliveries)
            .await
        {
            return Err(anyhow::anyhow!(
                "{turn_error:#}; additionally failed to persist delivered steering messages: {persist_error:#}"
            ));
        }
        Err(turn_error)
    }

    async fn persist_failed_steering_messages(
        &self,
        turn_id: TurnId,
        deliveries: &[SteeringDeliveryRecord],
    ) -> Result<()> {
        let mut history = self.session.history.lock().await;
        let messages = deliveries
            .iter()
            .map(|delivery| delivery.message.clone())
            .filter(|message| !history.iter().any(|stored| stored.id == message.id))
            .collect::<Vec<_>>();
        if messages.is_empty() {
            return Ok(());
        }
        if let Some(store) = &self.session.session_store {
            store
                .append_history(self.session.thread_id, Some(turn_id), &messages)
                .await?;
        }
        history.extend(messages);
        Ok(())
    }
}
