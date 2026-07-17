use std::{
    io::{self, BufReader, Read},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError, TrySendError},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::{
    Framing, ProcessSpec, ReceiveFrameError, ReceiveLimits,
    receive::{BufferedFrame, ReaderStatus, ReceiveBudget, compact_json_len},
};

/// Live persistent child process session.
#[derive(Debug)]
pub struct ProcessSession<F: Framing> {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: Receiver<BufferedFrame>,
    reader_status: ReaderStatus,
    next_request_id: i64,
    notifications: Vec<BufferedFrame>,
    framing: F,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl<F: Framing> ProcessSession<F> {
    pub fn spawn(spec: &ProcessSpec, framing: F) -> Result<Self> {
        Self::spawn_with_receive_limits(spec, framing, ReceiveLimits::default())
    }

    pub fn spawn_with_receive_limits(
        spec: &ProcessSpec,
        framing: F,
        receive_limits: ReceiveLimits,
    ) -> Result<Self> {
        receive_limits.validate()?;
        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .envs(&spec.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open child stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to open child stdout"))?;
        let stderr = child.stderr.take();

        let (stdout_rx, reader_status, stdout_thread) =
            spawn_stdout_reader(stdout, framing.clone(), receive_limits);
        let stderr_thread = stderr.map(spawn_stderr_drain);

        Ok(Self {
            child,
            stdin,
            stdout_rx,
            reader_status,
            next_request_id: 1,
            notifications: Vec::new(),
            framing,
            stdout_thread: Some(stdout_thread),
            stderr_thread,
        })
    }

    /// Sends one protocol-neutral frame to the child.
    pub fn send_frame(&mut self, message: Value) -> Result<()> {
        self.framing.write_frame(&mut self.stdin, &message)
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
        match self.stdout_rx.recv_timeout(timeout) {
            Ok(frame) => Ok(frame),
            Err(RecvTimeoutError::Timeout) => Err(self
                .reader_status
                .failure_if_any()
                .unwrap_or(ReceiveFrameError::Timeout { timeout })),
            Err(RecvTimeoutError::Disconnected) => Err(self.reader_status.failure()),
        }
    }

    fn try_recv_buffered_frame(
        &mut self,
    ) -> std::result::Result<Option<BufferedFrame>, ReceiveFrameError> {
        match self.stdout_rx.try_recv() {
            Ok(frame) => Ok(Some(frame)),
            Err(TryRecvError::Empty) => match self.reader_status.failure_if_any() {
                Some(error) => Err(error),
                None => Ok(None),
            },
            Err(TryRecvError::Disconnected) => Err(self.reader_status.failure()),
        }
    }

    fn next_request_id(&mut self) -> i64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        request_id
    }

    /// Terminates the child without creating a replacement session.
    pub fn terminate(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
            self.child.wait()?;
        }
        Ok(())
    }

    fn terminate_best_effort(&mut self) {
        let _ = self.terminate();
    }
}

impl<F: Framing> Drop for ProcessSession<F> {
    fn drop(&mut self) {
        self.terminate_best_effort();
        if let Some(thread) = self.stdout_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

fn spawn_stdout_reader<R, F>(
    reader: R,
    framing: F,
    receive_limits: ReceiveLimits,
) -> (Receiver<BufferedFrame>, ReaderStatus, JoinHandle<()>)
where
    R: Read + Send + 'static,
    F: Framing,
{
    let (tx, rx) = mpsc::sync_channel(receive_limits.max_buffered_frames());
    let budget = ReceiveBudget::new(receive_limits);
    let reader_status = ReaderStatus::default();
    let thread_status = reader_status.clone();
    let thread = std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        loop {
            let value = match framing.read_frame(&mut reader) {
                Ok(value) => value,
                Err(error) => {
                    thread_status.fail(error.to_string());
                    break;
                }
            };
            let serialized_bytes = match compact_json_len(&value) {
                Ok(serialized_bytes) => serialized_bytes,
                Err(error) => {
                    thread_status.fail(format!("failed to measure received JSON frame: {error}"));
                    break;
                }
            };
            let permit = match budget.reserve(serialized_bytes) {
                Ok(permit) => permit,
                Err(error) => {
                    thread_status.fail(error);
                    break;
                }
            };
            let frame = BufferedFrame::new(value, permit);
            match tx.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    thread_status.fail(format!(
                        "receive channel exceeded frame count limit: max {}",
                        receive_limits.max_buffered_frames()
                    ));
                    break;
                }
                Err(TrySendError::Disconnected(_)) => break,
            }
        }
    });
    (rx, reader_status, thread)
}

fn spawn_stderr_drain<R>(mut reader: R) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let _ = io::copy(&mut reader, &mut io::sink());
    })
}

fn is_notification(message: &Value) -> bool {
    message.get("id").is_none() && notification_method(message).is_some()
}

fn notification_method(message: &Value) -> Option<&str> {
    message.get("method").and_then(Value::as_str)
}
