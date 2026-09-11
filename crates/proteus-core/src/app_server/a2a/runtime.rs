use std::sync::Arc;

use a2a::{Part, TaskState};
use futures_util::FutureExt;

use super::state::{Run, Service};
use crate::app_server::{AgentAppServer, AppServerEvent};

impl Service {
    pub fn start(&self, run: Arc<Run>, text: String) {
        let service = self.clone();
        tokio::spawn(async move {
            let outcome = std::panic::AssertUnwindSafe(service.execute(&run, text))
                .catch_unwind()
                .await;
            if outcome.is_err() {
                // App-server owns a detached run. Stop it before publishing a
                // failure, including when this adapter itself panics.
                run.cancellation.cancel();
                if let Some(server) = run.context.server.get() {
                    server.close_runs().await;
                }
                service
                    .publish(
                        &run,
                        TaskState::Failed,
                        vec![Part::text("A2A runtime adapter panicked")],
                    )
                    .await;
            }
        });
    }

    async fn execute(&self, run: &Run, text: String) {
        let result = run
            .context
            .server
            .get_or_try_init(|| async {
                let server = AgentAppServer::launch(
                    self.config.clone(),
                    self.cwd.clone(),
                    self.config_path.as_deref(),
                )
                .await?;
                server.start_session().await?;
                Ok::<_, anyhow::Error>(server)
            })
            .await;
        let server = match result {
            Ok(server) => server,
            Err(error) => {
                let state = if run.cancellation.is_cancelled() {
                    TaskState::Canceled
                } else {
                    TaskState::Failed
                };
                self.publish(run, state, vec![Part::text(format!("{error:#}"))])
                    .await;
                return;
            }
        };
        if run.cancellation.is_cancelled() {
            self.publish(
                run,
                TaskState::Canceled,
                vec![Part::text("Canceled before execution")],
            )
            .await;
            return;
        }
        self.publish(run, TaskState::Working, vec![]).await;
        let mut events = server.subscribe_session();
        let execution = server.send_user_message_with_cancellation(text, run.cancellation.clone());
        tokio::pin!(execution);
        let result = loop {
            tokio::select! {
                biased;
                result = &mut execution => break result,
                event = events.recv() => match event {
                    Ok(AppServerEvent::PendingRequestsUpdated { .. }) => self.publish_pending(run).await,
                    Ok(_) => {}
                    Err(_) => {
                        run.cancellation.cancel();
                        break execution.await;
                    }
                }
            }
        };
        let (state, text) = match result {
            Ok(output) => (TaskState::Completed, output.text),
            Err(error)
                if crate::core::turn_settlement_status(&error, run.cancellation.is_cancelled())
                    == crate::core::TurnSettlementStatus::Canceled =>
            {
                (TaskState::Canceled, format!("{error:#}"))
            }
            Err(error) => (TaskState::Failed, format!("{error:#}")),
        };
        self.publish(run, state, vec![Part::text(text)]).await;
    }
}
