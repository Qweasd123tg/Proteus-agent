use std::{
    fmt,
    io::{self, BufReader, Read},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use crate::{
    DEFAULT_MAX_FRAME_BYTES, DEFAULT_MAX_QUEUED_CONTROL_WRITE_BYTES,
    DEFAULT_MAX_QUEUED_CONTROL_WRITES, DEFAULT_MAX_QUEUED_WRITE_BYTES, DEFAULT_MAX_QUEUED_WRITES,
    Framing, ProcessFrameWriter, ProcessLifecycle, ProcessSpec, ReceiveFrameError, ReceiveLimits,
    SendFrameError,
    receive::{BufferedFrame, ReaderStatus, ReceiveBudget, compact_json_len},
    writer::{WriterLimits, spawn_writer},
};
const READER_LIFECYCLE_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Bounds owned by the protocol-neutral process transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessTransportLimits {
    receive: ReceiveLimits,
    max_queued_writes: usize,
    max_queued_control_writes: usize,
    max_frame_bytes: usize,
    max_queued_write_bytes: usize,
    max_queued_control_write_bytes: usize,
}

impl ProcessTransportLimits {
    pub const fn new(receive: ReceiveLimits, max_queued_writes: usize) -> Self {
        Self {
            receive,
            max_queued_writes,
            max_queued_control_writes: DEFAULT_MAX_QUEUED_CONTROL_WRITES,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_queued_write_bytes: DEFAULT_MAX_QUEUED_WRITE_BYTES,
            max_queued_control_write_bytes: DEFAULT_MAX_QUEUED_CONTROL_WRITE_BYTES,
        }
    }

    pub const fn with_control_queue(mut self, max_queued_control_writes: usize) -> Self {
        self.max_queued_control_writes = max_queued_control_writes;
        self
    }

    pub const fn with_write_byte_limits(
        mut self,
        max_frame_bytes: usize,
        max_queued_write_bytes: usize,
        max_queued_control_write_bytes: usize,
    ) -> Self {
        self.max_frame_bytes = max_frame_bytes;
        self.max_queued_write_bytes = max_queued_write_bytes;
        self.max_queued_control_write_bytes = max_queued_control_write_bytes;
        self
    }

    pub const fn receive(self) -> ReceiveLimits {
        self.receive
    }

    pub const fn max_queued_writes(self) -> usize {
        self.max_queued_writes
    }

    pub const fn max_queued_control_writes(self) -> usize {
        self.max_queued_control_writes
    }

    pub const fn max_frame_bytes(self) -> usize {
        self.max_frame_bytes
    }

    pub const fn max_queued_write_bytes(self) -> usize {
        self.max_queued_write_bytes
    }

    pub const fn max_queued_control_write_bytes(self) -> usize {
        self.max_queued_control_write_bytes
    }

    fn validate(self) -> Result<()> {
        self.receive.validate()?;
        if self.max_queued_writes == 0 {
            bail!("transport max_queued_writes must be greater than zero");
        }
        if self.max_queued_control_writes == 0 {
            bail!("transport max_queued_control_writes must be greater than zero");
        }
        if self.max_frame_bytes == 0 {
            bail!("transport max_frame_bytes must be greater than zero");
        }
        if self.max_queued_write_bytes == 0 {
            bail!("transport max_queued_write_bytes must be greater than zero");
        }
        if self.max_queued_control_write_bytes == 0 {
            bail!("transport max_queued_control_write_bytes must be greater than zero");
        }
        Ok(())
    }
}

impl Default for ProcessTransportLimits {
    fn default() -> Self {
        Self::new(ReceiveLimits::default(), DEFAULT_MAX_QUEUED_WRITES)
    }
}

/// Single-consumer input half for one process generation.
#[derive(Debug)]
pub struct ProcessFrameReader {
    frames: Receiver<BufferedFrame>,
    status: ReaderStatus,
    lifecycle: ProcessLifecycle,
}

impl ProcessFrameReader {
    /// Waits for one frame without changing lifecycle state on timeout.
    pub fn recv_frame(
        &mut self,
        timeout: Duration,
    ) -> std::result::Result<Value, ReceiveFrameError> {
        self.recv_buffered_frame(timeout)
            .map(BufferedFrame::into_value)
    }

    pub(crate) fn recv_buffered_frame(
        &mut self,
        timeout: Duration,
    ) -> std::result::Result<BufferedFrame, ReceiveFrameError> {
        let started = std::time::Instant::now();
        loop {
            match self.frames.try_recv() {
                Ok(frame) => return Ok(frame),
                Err(TryRecvError::Disconnected) => return Err(self.receive_failure()),
                Err(TryRecvError::Empty) => {}
            }
            if let Some(error) = self.stopped_error() {
                return Err(error);
            }
            let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                return Err(ReceiveFrameError::Timeout { timeout });
            };
            if remaining.is_zero() {
                return Err(ReceiveFrameError::Timeout { timeout });
            }
            match self
                .frames
                .recv_timeout(remaining.min(READER_LIFECYCLE_POLL_INTERVAL))
            {
                Ok(frame) => return Ok(frame),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(self.receive_failure()),
            }
        }
    }

    /// Returns one queued frame immediately, or `None` while the generation
    /// and stdout reader remain live.
    pub fn try_recv_frame(&mut self) -> std::result::Result<Option<Value>, ReceiveFrameError> {
        self.try_recv_buffered_frame()
            .map(|frame| frame.map(BufferedFrame::into_value))
    }

    pub(crate) fn try_recv_buffered_frame(
        &mut self,
    ) -> std::result::Result<Option<BufferedFrame>, ReceiveFrameError> {
        match self.frames.try_recv() {
            Ok(frame) => Ok(Some(frame)),
            Err(TryRecvError::Empty) => match self.status.failure_if_any() {
                Some(error) => Err(error),
                None => match self.lifecycle.stopped_reason() {
                    Some(reason) => Err(ReceiveFrameError::ReaderStopped { reason }),
                    None => Ok(None),
                },
            },
            Err(TryRecvError::Disconnected) => Err(self.receive_failure()),
        }
    }

    fn stopped_error(&self) -> Option<ReceiveFrameError> {
        self.status.failure_if_any().or_else(|| {
            self.lifecycle
                .stopped_reason()
                .map(|reason| ReceiveFrameError::ReaderStopped { reason })
        })
    }

    fn receive_failure(&self) -> ReceiveFrameError {
        self.status.failure_if_any().unwrap_or_else(|| {
            self.lifecycle
                .stopped_reason()
                .map(|reason| ReceiveFrameError::ReaderStopped { reason })
                .unwrap_or_else(|| self.status.failure())
        })
    }
}

/// Protocol-neutral duplex stdio transport for one child-process generation.
///
/// The frame reader, bounded writer and lifecycle handle have independent
/// ownership. Protocol layers may clone the writer/lifecycle while keeping a
/// single reader responsible for classifying inbound frames.
pub struct ProcessTransport<F: Framing> {
    reader: ProcessFrameReader,
    writer: ProcessFrameWriter,
    lifecycle: ProcessLifecycle,
    stopping: Arc<AtomicBool>,
    reader_thread: Option<JoinHandle<()>>,
    writer_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    lifecycle_thread: Option<JoinHandle<()>>,
    _framing: std::marker::PhantomData<F>,
}

impl<F: Framing> fmt::Debug for ProcessTransport<F> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessTransport")
            .field("lifecycle", &self.lifecycle)
            .field("writer", &self.writer)
            .finish_non_exhaustive()
    }
}

impl<F: Framing> ProcessTransport<F> {
    pub fn spawn(spec: &ProcessSpec, framing: F) -> Result<Self> {
        Self::spawn_with_limits(spec, framing, ProcessTransportLimits::default())
    }

    pub fn spawn_with_limits(
        spec: &ProcessSpec,
        framing: F,
        limits: ProcessTransportLimits,
    ) -> Result<Self> {
        limits.validate()?;
        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        spec.apply_environment(&mut command)?;
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }

        let mut child = command.spawn()?;
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => return spawn_cleanup_error(child, "failed to open child stdin"),
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => return spawn_cleanup_error(child, "failed to open child stdout"),
        };
        let stderr = child.stderr.take();

        let stopping = Arc::new(AtomicBool::new(false));
        let (lifecycle, lifecycle_thread) = ProcessLifecycle::spawn(child);

        let (frame_tx, frame_rx) = mpsc::sync_channel(limits.receive.max_buffered_frames());
        let receive_budget = ReceiveBudget::new(limits.receive);
        let reader_status = ReaderStatus::default();
        let reader_thread = spawn_reader(
            stdout,
            framing.clone(),
            frame_tx,
            receive_budget,
            reader_status.clone(),
            Arc::clone(&stopping),
        );

        let (writer, writer_thread) = spawn_writer(
            stdin,
            framing,
            WriterLimits {
                max_queued_control_writes: limits.max_queued_control_writes,
                max_queued_writes: limits.max_queued_writes,
                max_frame_bytes: limits.max_frame_bytes,
                max_queued_control_write_bytes: limits.max_queued_control_write_bytes,
                max_queued_write_bytes: limits.max_queued_write_bytes,
            },
            Arc::clone(&stopping),
            lifecycle.clone(),
        );
        let stderr_thread = stderr.map(spawn_stderr_drain);

        Ok(Self {
            reader: ProcessFrameReader {
                frames: frame_rx,
                status: reader_status,
                lifecycle: lifecycle.clone(),
            },
            writer,
            lifecycle,
            stopping,
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
            stderr_thread,
            lifecycle_thread: Some(lifecycle_thread),
            _framing: std::marker::PhantomData,
        })
    }

    pub fn pid(&self) -> u32 {
        self.lifecycle.pid()
    }

    pub fn frame_writer(&self) -> ProcessFrameWriter {
        self.writer.clone()
    }

    pub fn lifecycle(&self) -> ProcessLifecycle {
        self.lifecycle.clone()
    }

    pub fn frame_reader(&mut self) -> &mut ProcessFrameReader {
        &mut self.reader
    }

    pub fn send_frame(&self, value: Value) -> std::result::Result<(), SendFrameError> {
        self.writer.send_frame(value)
    }

    pub fn send_control_frame(&self, value: Value) -> std::result::Result<(), SendFrameError> {
        self.writer.send_control_frame(value)
    }

    pub fn recv_frame(
        &mut self,
        timeout: Duration,
    ) -> std::result::Result<Value, ReceiveFrameError> {
        self.reader.recv_frame(timeout)
    }

    pub fn try_recv_frame(&mut self) -> std::result::Result<Option<Value>, ReceiveFrameError> {
        self.reader.try_recv_frame()
    }

    pub(crate) fn recv_buffered_frame(
        &mut self,
        timeout: Duration,
    ) -> std::result::Result<BufferedFrame, ReceiveFrameError> {
        self.reader.recv_buffered_frame(timeout)
    }

    pub(crate) fn try_recv_buffered_frame(
        &mut self,
    ) -> std::result::Result<Option<BufferedFrame>, ReceiveFrameError> {
        self.reader.try_recv_buffered_frame()
    }

    /// Stops the child generation and joins all transport workers. The method
    /// is idempotent; cloned writer/lifecycle handles observe the same stop.
    pub fn terminate(&mut self) -> Result<()> {
        self.stopping.store(true, Ordering::Release);
        let lifecycle_result = self.lifecycle.terminate().map(|_| ());
        let join_result = self.join_workers();
        lifecycle_result.and(join_result)
    }

    fn join_workers(&mut self) -> Result<()> {
        let mut first_error = None;
        join_one(&mut self.reader_thread, "stdout reader", &mut first_error);
        join_one(&mut self.writer_thread, "stdin writer", &mut first_error);
        join_one(&mut self.stderr_thread, "stderr drain", &mut first_error);
        join_one(
            &mut self.lifecycle_thread,
            "child lifecycle",
            &mut first_error,
        );
        first_error.map_or(Ok(()), Err)
    }
}

impl<F: Framing> Drop for ProcessTransport<F> {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

fn spawn_reader<R, F>(
    stdout: R,
    framing: F,
    frames: SyncSender<BufferedFrame>,
    budget: ReceiveBudget,
    status: ReaderStatus,
    stopping: Arc<AtomicBool>,
) -> JoinHandle<()>
where
    R: Read + Send + 'static,
    F: Framing,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let value = match framing.read_frame(&mut reader) {
                Ok(value) => value,
                Err(error) => {
                    let reason = if stopping.load(Ordering::Acquire) {
                        "process transport stopped".to_owned()
                    } else {
                        error.to_string()
                    };
                    status.fail(reason);
                    break;
                }
            };
            let serialized_bytes = match compact_json_len(&value) {
                Ok(serialized_bytes) => serialized_bytes,
                Err(error) => {
                    status.fail(format!("failed to measure received JSON frame: {error}"));
                    break;
                }
            };
            let permit = match budget.reserve(serialized_bytes) {
                Ok(permit) => permit,
                Err(error) => {
                    status.fail(error);
                    break;
                }
            };
            let frame = BufferedFrame::new(value, permit);
            match frames.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    status.fail(format!(
                        "receive channel exceeded frame count limit: max {}",
                        budget.limits().max_buffered_frames()
                    ));
                    break;
                }
                Err(TrySendError::Disconnected(_)) => break,
            }
        }
    })
}

fn spawn_stderr_drain<R>(mut stderr: R) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let _ = io::copy(&mut stderr, &mut io::sink());
    })
}

fn spawn_cleanup_error<T>(mut child: Child, message: &str) -> Result<T> {
    let _ = child.kill();
    let _ = child.wait();
    Err(anyhow!(message.to_owned()))
}

fn join_one(
    thread: &mut Option<JoinHandle<()>>,
    label: &str,
    first_error: &mut Option<anyhow::Error>,
) {
    let Some(thread) = thread.take() else {
        return;
    };
    if thread.join().is_err() && first_error.is_none() {
        *first_error = Some(anyhow!("process transport {label} thread panicked"));
    }
}
