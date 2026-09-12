//! SDK message transport over the shared bounded process lifecycle.

use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use proteus_process_host::{
    NewlineJsonFraming, ProcessFrameWriter, ProcessLifecycle, ProcessSpec, ProcessTransport,
};
use rmcp::{
    RoleClient,
    model::{ClientNotification, JsonRpcMessage},
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::Transport,
};
use tokio::task::JoinHandle;

type Incoming = RxJsonRpcMessage<RoleClient>;

#[derive(Clone)]
pub(super) struct ConnectionControl {
    lifecycle: ProcessLifecycle,
    error: Arc<Mutex<Option<String>>>,
}

impl ConnectionControl {
    pub(super) fn is_closed(&self) -> bool {
        self.error
            .lock()
            .expect("MCP transport error mutex")
            .is_some()
            || !matches!(self.lifecycle.try_exit(), Ok(None))
    }

    pub(super) fn error(&self) -> Option<String> {
        self.error
            .lock()
            .expect("MCP transport error mutex")
            .clone()
    }

    pub(super) async fn terminate(&self) -> anyhow::Result<()> {
        let lifecycle = self.lifecycle.clone();
        tokio::task::spawn_blocking(move || lifecycle.terminate()).await??;
        Ok(())
    }
}

pub(super) struct McpTransport {
    process: Arc<Mutex<ProcessTransport<NewlineJsonFraming>>>,
    writer: ProcessFrameWriter,
    control: ConnectionControl,
    // rmcp may drop receive() in select! when sending another message. Keep
    // ownership of the pending read so its consumed frame cannot be lost.
    pending_receive: Option<JoinHandle<io::Result<Incoming>>>,
}

impl McpTransport {
    pub(super) fn spawn(spec: &ProcessSpec, max_response_bytes: usize) -> anyhow::Result<Self> {
        let process = ProcessTransport::spawn(spec, NewlineJsonFraming::new(max_response_bytes))?;
        Ok(Self {
            writer: process.frame_writer(),
            control: ConnectionControl {
                lifecycle: process.lifecycle(),
                error: Arc::new(Mutex::new(None)),
            },
            process: Arc::new(Mutex::new(process)),
            pending_receive: None,
        })
    }

    pub(super) fn control(&self) -> ConnectionControl {
        self.control.clone()
    }
}

impl Transport<RoleClient> for McpTransport {
    type Error = io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let writer = self.writer.clone();
        let is_cancellation = matches!(&item, JsonRpcMessage::Notification(notification)
            if matches!(&notification.notification, ClientNotification::CancelledNotification(_)));
        async move {
            let value = serde_json::to_value(item).map_err(io::Error::other)?;
            tokio::task::spawn_blocking(move || {
                // The priority lane remains bounded and cannot get stuck
                // behind a full ordinary request queue during cancellation.
                if is_cancellation {
                    writer.send_control_frame(value)
                } else {
                    writer.send_frame(value)
                }
                .map_err(io::Error::other)
            })
            .await
            .map_err(io::Error::other)?
        }
    }

    async fn receive(&mut self) -> Option<Incoming> {
        let pending = self.pending_receive.get_or_insert_with(|| {
            let process = self.process.clone();
            tokio::task::spawn_blocking(move || {
                let value = process
                    .lock()
                    .expect("MCP process mutex")
                    .recv_frame(Duration::MAX)
                    .map_err(io::Error::other)?;
                serde_json::from_value(value).map_err(io::Error::other)
            })
        });
        let result = pending.await;
        self.pending_receive = None;
        match result.map_err(io::Error::other).and_then(|result| result) {
            Ok(message) => Some(message),
            Err(error) => {
                *self
                    .control
                    .error
                    .lock()
                    .expect("MCP transport error mutex") = Some(error.to_string());
                None
            }
        }
    }

    async fn close(&mut self) -> io::Result<()> {
        self.control.terminate().await.map_err(io::Error::other)
    }
}

impl Drop for McpTransport {
    fn drop(&mut self) {
        // Also covers a dropped initialize future. Termination wakes the
        // blocking read, releases its Arc and lets ProcessTransport join IO.
        let _ = self.control.lifecycle.terminate();
    }
}
