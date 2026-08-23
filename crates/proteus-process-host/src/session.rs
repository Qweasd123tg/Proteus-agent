use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::{
    Framing, ProcessFrameWriter, ProcessLifecycle, ProcessSpec, ProcessTransport,
    ProcessTransportLimits, ReceiveFrameError, ReceiveLimits, receive::BufferedFrame,
};

/// Sequential JSON-RPC-style facade over a protocol-neutral duplex transport.
///
/// MCP and LSP keep their existing
/// single-caller semantics here. New multiplexed protocol layers use
/// [`ProcessTransport`] directly instead of duplicating child ownership.
#[derive(Debug)]
pub struct ProcessSession<F: Framing> {
    transport: ProcessTransport<F>,
    next_request_id: i64,
    notifications: Vec<BufferedFrame>,
}

impl<F: Framing> ProcessSession<F> {
    pub fn spawn(spec: &ProcessSpec, framing: F) -> Result<Self> {
        Self::spawn_with_transport_limits(spec, framing, ProcessTransportLimits::default())
    }

    pub fn spawn_with_receive_limits(
        spec: &ProcessSpec,
        framing: F,
        receive_limits: ReceiveLimits,
    ) -> Result<Self> {
        Self::spawn_with_transport_limits(
            spec,
            framing,
            ProcessTransportLimits::new(receive_limits, crate::DEFAULT_MAX_QUEUED_WRITES),
        )
    }

    pub fn spawn_with_transport_limits(
        spec: &ProcessSpec,
        framing: F,
        limits: ProcessTransportLimits,
    ) -> Result<Self> {
        Ok(Self {
            transport: ProcessTransport::spawn_with_limits(spec, framing, limits)?,
            next_request_id: 1,
            notifications: Vec::new(),
        })
    }

    pub fn pid(&self) -> u32 {
        self.transport.pid()
    }

    pub fn lifecycle(&self) -> ProcessLifecycle {
        self.transport.lifecycle()
    }

    pub fn frame_writer(&self) -> ProcessFrameWriter {
        self.transport.frame_writer()
    }

    /// Sends one protocol-neutral frame to the child.
    pub fn send_frame(&mut self, message: Value) -> Result<()> {
        self.transport.send_frame(message).map_err(Into::into)
    }

    /// Waits for one protocol-neutral frame without changing process state on
    /// timeout. The caller decides whether to continue, abort, or terminate.
    pub fn recv_frame(
        &mut self,
        timeout: Duration,
    ) -> std::result::Result<Value, ReceiveFrameError> {
        self.recv_buffered_frame(timeout)
            .map(BufferedFrame::into_value)
    }

    /// Returns a queued frame immediately, or `None` while the reader remains
    /// live and no frame is ready.
    pub fn try_recv_frame(&mut self) -> std::result::Result<Option<Value>, ReceiveFrameError> {
        self.try_recv_buffered_frame()
            .map(|frame| frame.map(BufferedFrame::into_value))
    }

    pub fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let request_id = self.next_request_id();
        let request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        self.send_frame(request)?;
        self.recv_response(request_id, timeout)
    }

    pub fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send_frame(notification)
    }

    pub fn drain_notifications(&mut self) -> Vec<Value> {
        self.notifications
            .drain(..)
            .map(BufferedFrame::into_value)
            .collect()
    }

    pub fn wait_notification(&mut self, method: &str, timeout: Duration) -> Result<Value> {
        if let Some(index) = self
            .notifications
            .iter()
            .position(|message| notification_method(message.value()) == Some(method))
        {
            return Ok(self.notifications.remove(index).into_value());
        }

        let started = Instant::now();
        loop {
            let message = self.recv_frame_before(started, timeout, "notification")?;
            if notification_method(message.value()) == Some(method) {
                return Ok(message.into_value());
            }
            if is_notification(message.value()) {
                self.notifications.push(message);
            }
        }
    }

    fn recv_response(&mut self, expected_id: i64, timeout: Duration) -> Result<Value> {
        let started = Instant::now();
        loop {
            let message = self.recv_frame_before(started, timeout, "response")?;
            if is_notification(message.value()) {
                self.notifications.push(message);
                continue;
            }

            let Some(id) = message.value().get("id") else {
                continue;
            };
            if id != &json!(expected_id) {
                bail!("response id {id} did not match expected id {expected_id}");
            }
            if let Some(error) = message.value().get("error") {
                bail!("JSON-RPC error response: {error}");
            }
            return message
                .value()
                .get("result")
                .cloned()
                .ok_or_else(|| anyhow!("JSON-RPC response missing result"));
        }
    }

    fn recv_frame_before(
        &mut self,
        started: Instant,
        timeout: Duration,
        expected: &str,
    ) -> Result<BufferedFrame> {
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            self.terminate_best_effort();
            bail!(
                "child did not send {expected} within {}ms",
                timeout.as_millis()
            );
        }

        match self.recv_buffered_frame(timeout - elapsed) {
            Ok(frame) => Ok(frame),
            Err(ReceiveFrameError::Timeout { .. }) => {
                self.terminate_best_effort();
                bail!(
                    "child did not send {expected} within {}ms",
                    timeout.as_millis()
                );
            }
            Err(error) => Err(error.into()),
        }
    }

    fn recv_buffered_frame(
        &mut self,
        timeout: Duration,
    ) -> std::result::Result<BufferedFrame, ReceiveFrameError> {
        self.transport.recv_buffered_frame(timeout)
    }

    fn try_recv_buffered_frame(
        &mut self,
    ) -> std::result::Result<Option<BufferedFrame>, ReceiveFrameError> {
        self.transport.try_recv_buffered_frame()
    }

    fn next_request_id(&mut self) -> i64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        request_id
    }

    /// Terminates the child without creating a replacement session.
    pub fn terminate(&mut self) -> Result<()> {
        self.transport.terminate()
    }

    fn terminate_best_effort(&mut self) {
        let _ = self.terminate();
    }
}

fn is_notification(message: &Value) -> bool {
    message.get("id").is_none() && notification_method(message).is_some()
}

fn notification_method(message: &Value) -> Option<&str> {
    message.get("method").and_then(Value::as_str)
}
