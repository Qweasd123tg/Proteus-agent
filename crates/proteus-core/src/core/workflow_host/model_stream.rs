//! A workflow-scoped pump keeps model IO moving while the worker handles an
//! item (including an approval or a tool invocation). Only completed items cross
//! the workflow boundary; token presentation remains owned by BoundModel.
use anyhow::{Result, anyhow, ensure};
use futures_util::StreamExt;
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
};

use crate::{
    contracts::{
        AgentWorkflowContext, ModelCallOrigin, WorkflowModelStreamCursor, WorkflowModelStreamItem,
    },
    core::model_call_scope::with_model_call_origin,
    model_standard::{CanonicalModelRequest, ModelFailure, ModelStreamEvent},
};

#[derive(Default)]
pub(super) struct ModelStreams {
    active: Mutex<Option<ActiveStream>>,
    closing: crate::contracts::CancellationToken,
}

struct ActiveStream {
    id: uuid::Uuid,
    items: mpsc::Receiver<WorkflowModelStreamItem>,
    task: JoinHandle<()>,
}

impl Drop for ActiveStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ModelStreams {
    pub(super) async fn start(
        &self,
        ctx: &AgentWorkflowContext,
        request: CanonicalModelRequest,
    ) -> Result<WorkflowModelStreamCursor> {
        let mut active = self
            .active
            .try_lock()
            .map_err(|_| anyhow!("model stream operation already pending"))?;
        ensure!(
            active.is_none(),
            "workflow already has an active model stream"
        );
        ensure!(
            !self.closing.is_cancelled(),
            "workflow model streams are closed"
        );
        let id = uuid::Uuid::new_v4();
        // Completed items only. Bounded backpressure does not depend on tokens
        // and preserves the provider's ordering without an unbounded queue.
        let (sender, items) = mpsc::channel(64);
        let ctx = ctx.clone();
        let task = tokio::spawn(with_model_call_origin(
            ModelCallOrigin::Direct,
            async move {
                let result = async {
                    let mut stream = ctx.execution.model.stream(request).await?;
                    while let Some(event) = stream.next().await {
                        let item = match event? {
                            ModelStreamEvent::MessageCompleted { message } => {
                                WorkflowModelStreamItem::MessageCompleted { message }
                            }
                            ModelStreamEvent::Response { response } => {
                                WorkflowModelStreamItem::Response { response }
                            }
                            ModelStreamEvent::Error { failure } => {
                                WorkflowModelStreamItem::Error { failure }
                            }
                            _ => continue,
                        };
                        let terminal =
                            !matches!(&item, WorkflowModelStreamItem::MessageCompleted { .. });
                        if sender.send(item).await.is_err() || terminal {
                            return Ok(());
                        }
                    }
                    Err(anyhow!("model stream ended without a terminal outcome"))
                }
                .await;
                if let Err(error) = result {
                    let _ = sender
                        .send(WorkflowModelStreamItem::Error {
                            failure: ModelFailure::from_error(&error),
                        })
                        .await;
                }
            },
        ));
        *active = Some(ActiveStream { id, items, task });
        Ok(WorkflowModelStreamCursor { stream_id: id })
    }

    pub(super) async fn next(
        &self,
        cursor: WorkflowModelStreamCursor,
    ) -> Result<WorkflowModelStreamItem> {
        let mut active = self
            .active
            .try_lock()
            .map_err(|_| anyhow!("model stream operation already pending"))?;
        let stream = active
            .as_mut()
            .ok_or_else(|| anyhow!("no active model stream"))?;
        ensure!(stream.id == cursor.stream_id, "unknown model stream cursor");
        let item = tokio::select! {
            biased;
            _ = self.closing.cancelled() => return Err(anyhow!("workflow model streams are closed")),
            item = stream.items.recv() => item.ok_or_else(|| anyhow!("model stream pump stopped without an outcome"))?,
        };
        if !matches!(item, WorkflowModelStreamItem::MessageCompleted { .. }) {
            active.take();
        }
        Ok(item)
    }

    pub(super) async fn close(&self) -> bool {
        self.closing.cancel();
        let Some(mut stream) = self.active.lock().await.take() else {
            return false;
        };
        stream.task.abort();
        let _ = (&mut stream.task).await;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cursor_validation_and_cleanup_do_not_leave_a_pending_next() {
        let streams = ModelStreams::default();
        let cursor = WorkflowModelStreamCursor {
            stream_id: uuid::Uuid::new_v4(),
        };
        let (sender, items) = mpsc::channel(1);
        *streams.active.lock().await = Some(ActiveStream {
            id: cursor.stream_id,
            items,
            task: tokio::spawn(std::future::pending()),
        });
        let foreign = WorkflowModelStreamCursor {
            stream_id: uuid::Uuid::new_v4(),
        };
        assert!(
            streams
                .next(foreign)
                .await
                .unwrap_err()
                .to_string()
                .contains("unknown model stream cursor")
        );
        let mut pending = Box::pin(streams.next(cursor.clone()));
        assert!(futures_util::poll!(&mut pending).is_pending());
        assert!(
            streams
                .next(cursor.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("operation already pending")
        );
        // Close must wake a next holding the slot mutex, even without root cancel.
        let (pending, closed) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(pending, streams.close())
        })
        .await
        .unwrap();
        assert!(pending.is_err());
        assert!(closed);
        assert!(sender.is_closed());
        assert!(streams.next(cursor).await.is_err());
        assert!(!streams.close().await);
    }
}
