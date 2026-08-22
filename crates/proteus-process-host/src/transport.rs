use std::{
    error::Error,
    fmt,
    io::{self, BufReader, Read},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use crate::{
    Framing, ProcessLifecycle, ProcessSpec, ReceiveFrameError, ReceiveLimits,
    receive::{BufferedFrame, ReaderStatus, ReceiveBudget, compact_json_len},
};

/// Default number of frames that may wait for the dedicated stdin writer.
pub const DEFAULT_MAX_QUEUED_WRITES: usize = 256;

const WRITER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const READER_LIFECYCLE_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Bounds owned by the protocol-neutral process transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessTransportLimits {
    receive: ReceiveLimits,
    max_queued_writes: usize,
}

impl ProcessTransportLimits {
    pub const fn new(receive: ReceiveLimits, max_queued_writes: usize) -> Self {
        Self {
            receive,
            max_queued_writes,
        }
    }

    pub const fn receive(self) -> ReceiveLimits {
        self.receive
    }

    pub const fn max_queued_writes(self) -> usize {
        self.max_queued_writes
    }

    fn validate(self) -> Result<()> {
        self.receive.validate()?;
        if self.max_queued_writes == 0 {
            bail!("transport max_queued_writes must be greater than zero");
        }
        Ok(())
    }
}

impl Default for ProcessTransportLimits {
    fn default() -> Self {
        Self::new(ReceiveLimits::default(), DEFAULT_MAX_QUEUED_WRITES)
    }
}

/// A protocol-neutral failure while writing a frame.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SendFrameError {
    QueueFull { max_queued_writes: usize },
    WriterStopped { reason: String },
}

impl fmt::Display for SendFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull { max_queued_writes } => write!(
                formatter,
                "process writer queue is full: max {max_queued_writes} frames"
            ),
            Self::WriterStopped { reason } => formatter.write_str(reason),
        }
    }
}

impl Error for SendFrameError {}

#[derive(Clone, Debug, Default)]
struct WriterStatus {
    failure: Arc<Mutex<Option<SendFrameError>>>,
}

impl WriterStatus {
    fn fail(&self, reason: impl Into<String>) -> SendFrameError {
        let mut failure = self.failure.lock().expect("writer status mutex poisoned");
        if failure.is_none() {
            *failure = Some(SendFrameError::WriterStopped {
                reason: reason.into(),
            });
        }
        failure
            .clone()
            .expect("writer status must contain a failure")
    }

    fn failure(&self) -> SendFrameError {
        self.failure
            .lock()
            .expect("writer status mutex poisoned")
            .clone()
            .unwrap_or_else(|| SendFrameError::WriterStopped {
                reason: "child stdin writer stopped".to_owned(),
            })
    }
}

enum WriterCommand {
    Frame {
        value: Value,
        completion: SyncSender<std::result::Result<(), SendFrameError>>,
    },
}

/// Cloneable bounded handle for writing whole frames to one process
/// generation. A dedicated writer thread is the sole owner of child stdin, so
/// concurrent callers cannot interleave bytes from different frames.
#[derive(Clone)]
pub struct ProcessFrameWriter {
    commands: SyncSender<WriterCommand>,
    status: WriterStatus,
    stopping: Arc<AtomicBool>,
    lifecycle: ProcessLifecycle,
    max_queued_writes: usize,
}

impl fmt::Debug for ProcessFrameWriter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessFrameWriter")
            .field("pid", &self.lifecycle.pid())
            .field("max_queued_writes", &self.max_queued_writes)
            .field("stopping", &self.stopping.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl ProcessFrameWriter {
    pub fn send_frame(&self, value: Value) -> std::result::Result<(), SendFrameError> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(self.status.failure());
        }
        if let Some(reason) = self.lifecycle.stopped_reason() {
            return Err(self.status.fail(reason));
        }

        let (completion_tx, completion_rx) = mpsc::sync_channel(1);
        let command = WriterCommand::Frame {
            value,
            completion: completion_tx,
        };
        match self.commands.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(SendFrameError::QueueFull {
                    max_queued_writes: self.max_queued_writes,
                });
            }
            Err(TrySendError::Disconnected(_)) => return Err(self.status.failure()),
        }

        match completion_rx.recv() {
            Ok(result) => result,
            Err(_) => Err(self.status.failure()),
        }
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

        let (writer_tx, writer_rx) = mpsc::sync_channel(limits.max_queued_writes);
        let writer_status = WriterStatus::default();
        let writer_thread = spawn_writer(
            stdin,
            framing,
            writer_rx,
            writer_status.clone(),
            Arc::clone(&stopping),
        );
        let stderr_thread = stderr.map(spawn_stderr_drain);

        Ok(Self {
            reader: ProcessFrameReader {
                frames: frame_rx,
                status: reader_status,
                lifecycle: lifecycle.clone(),
            },
            writer: ProcessFrameWriter {
                commands: writer_tx,
                status: writer_status,
                stopping: Arc::clone(&stopping),
                lifecycle: lifecycle.clone(),
                max_queued_writes: limits.max_queued_writes,
            },
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

fn spawn_writer<W, F>(
    mut stdin: W,
    framing: F,
    commands: Receiver<WriterCommand>,
    status: WriterStatus,
    stopping: Arc<AtomicBool>,
) -> JoinHandle<()>
where
    W: io::Write + Send + 'static,
    F: Framing,
{
    thread::spawn(move || {
        while !stopping.load(Ordering::Acquire) {
            let command = match commands.recv_timeout(WRITER_POLL_INTERVAL) {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            match command {
                WriterCommand::Frame { value, completion } => {
                    if stopping.load(Ordering::Acquire) {
                        let _ = completion.send(Err(status.fail("process transport stopped")));
                        break;
                    }
                    match framing.write_frame(&mut stdin, &value) {
                        Ok(()) => {
                            let _ = completion.send(Ok(()));
                        }
                        Err(error) => {
                            let failure = status
                                .fail(format!("failed to write frame to child stdin: {error}"));
                            let _ = completion.send(Err(failure));
                            break;
                        }
                    }
                }
            }
        }
        stopping.store(true, Ordering::Release);
        status.fail("child stdin writer stopped");
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
