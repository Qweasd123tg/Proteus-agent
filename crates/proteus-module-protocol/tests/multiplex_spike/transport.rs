//! Test-only duplex stdio transport for the component-v3 multiplexing spike.
//!
//! `ProcessSession` deliberately has one synchronous caller and one stdin
//! writer.  That is the correct production-v2 shape, but it would hide the
//! scheduling question this spike needs to answer.  This transport keeps the
//! child lifecycle in the broker thread while dedicated readers/writers own the
//! stdio handles.  It must not be promoted into `proteus-process-host` without
//! a separate contract review.

use std::{
    fmt,
    io::{self, BufReader, Read},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use proteus_process_host::{Framing, NewlineJsonFraming, ProcessSpec, ReceiveLimits};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lane {
    Control,
    Data,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SendError {
    Full { lane: Lane },
    FrameTooLarge { bytes: usize, max: usize },
    Stopped,
}

impl fmt::Display for SendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full { lane } => write!(formatter, "{lane:?} outbound queue is full"),
            Self::FrameTooLarge { bytes, max } => {
                write!(formatter, "outbound frame is {bytes} bytes, max {max}")
            }
            Self::Stopped => formatter.write_str("duplex process transport is stopped"),
        }
    }
}

impl std::error::Error for SendError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReceiveError {
    Timeout { timeout: Duration },
    Stopped { reason: String },
    Resource { reason: String },
}

impl ReceiveError {
    fn stopped(reason: impl Into<String>) -> Self {
        Self::Stopped {
            reason: reason.into(),
        }
    }

    fn resource(reason: impl Into<String>) -> Self {
        Self::Resource {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ReceiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { timeout } => write!(
                formatter,
                "child did not send a frame within {}ms",
                timeout.as_millis()
            ),
            Self::Stopped { reason } | Self::Resource { reason } => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for ReceiveError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportLimits {
    pub receive: ReceiveLimits,
    pub control_queue: usize,
    pub data_queue: usize,
    pub max_frame_bytes: usize,
}

impl TransportLimits {
    pub const fn new(receive: ReceiveLimits, control_queue: usize, data_queue: usize) -> Self {
        Self {
            receive,
            control_queue,
            data_queue,
            max_frame_bytes: 1024 * 1024,
        }
    }

    fn validate(self) -> Result<()> {
        if self.receive.max_buffered_frames() == 0 {
            bail!("transport receive max_buffered_frames must be greater than zero");
        }
        if self.receive.max_buffered_bytes() == 0 {
            bail!("transport receive max_buffered_bytes must be greater than zero");
        }
        if self.control_queue == 0 {
            bail!("transport control_queue must be greater than zero");
        }
        if self.data_queue == 0 {
            bail!("transport data_queue must be greater than zero");
        }
        if self.max_frame_bytes == 0 {
            bail!("transport max_frame_bytes must be greater than zero");
        }
        Ok(())
    }
}

impl Default for TransportLimits {
    fn default() -> Self {
        Self::new(ReceiveLimits::default(), 64, 256)
    }
}

#[derive(Debug)]
struct InboundFrame {
    value: Value,
    bytes: usize,
}

#[derive(Clone, Debug, Default)]
struct ReaderStatus {
    error: Arc<Mutex<Option<ReceiveError>>>,
}

impl ReaderStatus {
    fn set_once(&self, error: ReceiveError) {
        let mut current = self.error.lock().expect("reader status mutex poisoned");
        if current.is_none() {
            *current = Some(error);
        }
    }

    fn get(&self) -> Option<ReceiveError> {
        self.error
            .lock()
            .expect("reader status mutex poisoned")
            .clone()
    }

    fn stopped(&self) -> ReceiveError {
        self.get()
            .unwrap_or_else(|| ReceiveError::stopped("child stdout reader stopped"))
    }
}

#[derive(Clone, Debug)]
struct ReceiveBudget {
    limits: ReceiveLimits,
    state: Arc<Mutex<ReceiveBudgetState>>,
}

#[derive(Debug, Default)]
struct ReceiveBudgetState {
    frames: usize,
    bytes: usize,
}

impl ReceiveBudget {
    fn new(limits: ReceiveLimits) -> Self {
        Self {
            limits,
            state: Arc::new(Mutex::new(ReceiveBudgetState::default())),
        }
    }

    fn reserve(&self, bytes: usize) -> std::result::Result<(), ReceiveError> {
        let mut state = self.state.lock().expect("receive budget mutex poisoned");
        let frames = state.frames.saturating_add(1);
        if frames > self.limits.max_buffered_frames() {
            return Err(ReceiveError::resource(format!(
                "receive buffer exceeded frame count limit: attempted {frames}, max {}",
                self.limits.max_buffered_frames()
            )));
        }

        let total_bytes = state.bytes.saturating_add(bytes);
        if total_bytes > self.limits.max_buffered_bytes() {
            return Err(ReceiveError::resource(format!(
                "receive buffer exceeded aggregate byte limit: attempted {total_bytes}, max {}",
                self.limits.max_buffered_bytes()
            )));
        }
        state.frames = frames;
        state.bytes = total_bytes;
        Ok(())
    }

    fn release(&self, bytes: usize) {
        let mut state = self.state.lock().expect("receive budget mutex poisoned");
        debug_assert!(state.frames > 0);
        debug_assert!(state.bytes >= bytes);
        state.frames = state.frames.saturating_sub(1);
        state.bytes = state.bytes.saturating_sub(bytes);
    }
}

const DISPATCH_QUEUED: u8 = 0;
const DISPATCH_WRITING: u8 = 1;
const DISPATCH_WRITTEN: u8 = 2;
const DISPATCH_CANCELED: u8 = 3;

#[derive(Clone, Debug)]
pub struct DispatchToken {
    state: Arc<AtomicU8>,
}

impl DispatchToken {
    fn queued() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(DISPATCH_QUEUED)),
        }
    }

    pub fn cancel_before_write(&self) -> bool {
        self.state
            .compare_exchange(
                DISPATCH_QUEUED,
                DISPATCH_CANCELED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub fn is_written(&self) -> bool {
        self.state.load(Ordering::Acquire) == DISPATCH_WRITTEN
    }

    fn begin_write(&self) -> bool {
        self.state
            .compare_exchange(
                DISPATCH_QUEUED,
                DISPATCH_WRITING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn finish_write(&self) {
        self.state.store(DISPATCH_WRITTEN, Ordering::Release);
    }
}

enum WriterCommand {
    Frame {
        value: Value,
        dispatch: Option<DispatchToken>,
    },
    Shutdown,
}

/// A process whose child lifecycle remains with the caller while its stdout and
/// stdin are serviced by bounded background threads.
#[derive(Debug)]
pub struct DuplexProcess {
    child: Option<Child>,
    pid: u32,
    incoming_rx: Receiver<InboundFrame>,
    receive_budget: ReceiveBudget,
    reader_status: ReaderStatus,
    control_tx: SyncSender<WriterCommand>,
    data_tx: SyncSender<WriterCommand>,
    max_frame_bytes: usize,
    stopping: Arc<AtomicBool>,
    reader_thread: Option<JoinHandle<()>>,
    writer_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl DuplexProcess {
    pub fn spawn(spec: &ProcessSpec, limits: TransportLimits) -> Result<Self> {
        limits.validate()?;

        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .envs(spec.resolved_environment()?);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }

        let mut child = command.spawn()?;
        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open child stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to open child stdout"))?;
        let stderr = child.stderr.take();

        let stopping = Arc::new(AtomicBool::new(false));
        let receive_budget = ReceiveBudget::new(limits.receive);
        let reader_status = ReaderStatus::default();
        let (incoming_tx, incoming_rx) = mpsc::sync_channel(limits.receive.max_buffered_frames());
        let reader_thread = spawn_reader(
            stdout,
            incoming_tx,
            receive_budget.clone(),
            reader_status.clone(),
            Arc::clone(&stopping),
        );
        let (control_tx, control_rx) = mpsc::sync_channel(limits.control_queue);
        let (data_tx, data_rx) = mpsc::sync_channel(limits.data_queue);
        let writer_thread = spawn_writer(stdin, control_rx, data_rx, Arc::clone(&stopping));
        let stderr_thread = stderr.map(spawn_stderr_drain);

        Ok(Self {
            child: Some(child),
            pid,
            incoming_rx,
            receive_budget,
            reader_status,
            control_tx,
            data_tx,
            max_frame_bytes: limits.max_frame_bytes,
            stopping,
            reader_thread: Some(reader_thread),
            writer_thread: Some(writer_thread),
            stderr_thread,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn send_control(&self, frame: Value) -> std::result::Result<(), SendError> {
        self.enqueue_frame(&self.control_tx, Lane::Control, frame, None)
    }

    pub fn send_data(&self, frame: Value) -> std::result::Result<DispatchToken, SendError> {
        let dispatch = DispatchToken::queued();
        self.enqueue_frame(&self.data_tx, Lane::Data, frame, Some(dispatch.clone()))?;
        Ok(dispatch)
    }

    pub fn recv(&self, timeout: Duration) -> std::result::Result<Value, ReceiveError> {
        match self.incoming_rx.recv_timeout(timeout) {
            Ok(frame) => Ok(self.take_frame(frame)),
            Err(RecvTimeoutError::Timeout) => Err(self
                .reader_status
                .get()
                .unwrap_or(ReceiveError::Timeout { timeout })),
            Err(RecvTimeoutError::Disconnected) => Err(self.reader_status.stopped()),
        }
    }

    pub fn try_recv(&self) -> std::result::Result<Option<Value>, ReceiveError> {
        match self.incoming_rx.try_recv() {
            Ok(frame) => Ok(Some(self.take_frame(frame))),
            Err(TryRecvError::Empty) => match self.reader_status.get() {
                Some(error) => Err(error),
                None => Ok(None),
            },
            Err(TryRecvError::Disconnected) => Err(self.reader_status.stopped()),
        }
    }

    /// Kills the child first, which unblocks a writer stuck in `write_all`, and
    /// only then stops and joins the I/O workers.
    pub fn terminate(&mut self) -> Result<()> {
        let child_result = (|| -> Result<()> {
            if let Some(child) = self.child.as_mut()
                && child.try_wait()?.is_none()
            {
                child.kill()?;
                child.wait()?;
            }
            Ok(())
        })();
        self.stop_workers();
        child_result
    }

    fn enqueue_frame(
        &self,
        sender: &SyncSender<WriterCommand>,
        lane: Lane,
        frame: Value,
        dispatch: Option<DispatchToken>,
    ) -> std::result::Result<(), SendError> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(SendError::Stopped);
        }
        let bytes = serde_json::to_vec(&frame).map_or(usize::MAX, |encoded| encoded.len());
        if bytes > self.max_frame_bytes {
            return Err(SendError::FrameTooLarge {
                bytes,
                max: self.max_frame_bytes,
            });
        }
        match sender.try_send(WriterCommand::Frame {
            value: frame,
            dispatch,
        }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(SendError::Full { lane }),
            Err(TrySendError::Disconnected(_)) => Err(SendError::Stopped),
        }
    }

    fn take_frame(&self, frame: InboundFrame) -> Value {
        self.receive_budget.release(frame.bytes);
        frame.value
    }

    fn stop_workers(&mut self) {
        if !self.stopping.swap(true, Ordering::AcqRel) {
            let _ = self.control_tx.try_send(WriterCommand::Shutdown);
        }
        if let Some(thread) = self.reader_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.writer_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for DuplexProcess {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

fn spawn_reader<R>(
    stdout: R,
    incoming_tx: SyncSender<InboundFrame>,
    budget: ReceiveBudget,
    status: ReaderStatus,
    stopping: Arc<AtomicBool>,
) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let framing = NewlineJsonFraming::default();
        let mut reader = BufReader::new(stdout);
        loop {
            let value = match framing.read_frame(&mut reader) {
                Ok(value) => value,
                Err(error) => {
                    if !stopping.load(Ordering::Acquire) {
                        status.set_once(ReceiveError::stopped(error.to_string()));
                    }
                    break;
                }
            };
            let bytes = match serde_json::to_vec(&value) {
                Ok(serialized) => serialized.len(),
                Err(error) => {
                    status.set_once(ReceiveError::stopped(format!(
                        "failed to measure received JSON frame: {error}"
                    )));
                    break;
                }
            };
            if let Err(error) = budget.reserve(bytes) {
                status.set_once(error);
                break;
            }
            let frame = InboundFrame { value, bytes };
            match incoming_tx.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(frame)) => {
                    budget.release(frame.bytes);
                    status.set_once(ReceiveError::resource(format!(
                        "receive channel exceeded frame count limit: max {}",
                        budget.limits.max_buffered_frames()
                    )));
                    break;
                }
                Err(TrySendError::Disconnected(frame)) => {
                    budget.release(frame.bytes);
                    break;
                }
            }
        }
    })
}

fn spawn_writer<W>(
    mut stdin: W,
    control_rx: Receiver<WriterCommand>,
    data_rx: Receiver<WriterCommand>,
    stopping: Arc<AtomicBool>,
) -> JoinHandle<()>
where
    W: io::Write + Send + 'static,
{
    thread::spawn(move || {
        let framing = NewlineJsonFraming::default();
        while !stopping.load(Ordering::Acquire) {
            let command = match next_writer_command(&control_rx, &data_rx) {
                Some(command) => command,
                None => break,
            };
            match command {
                WriterCommand::Shutdown => break,
                WriterCommand::Frame { value, dispatch } => {
                    if dispatch
                        .as_ref()
                        .is_some_and(|dispatch| !dispatch.begin_write())
                    {
                        continue;
                    }
                    if framing.write_frame(&mut stdin, &value).is_err() {
                        break;
                    }
                    if let Some(dispatch) = dispatch {
                        dispatch.finish_write();
                    }
                }
            }
        }
        stopping.store(true, Ordering::Release);
    })
}

/// Checks the control queue before every frame.  A control command which was
/// already queued therefore cannot be overtaken by a data frame; a command
/// racing that check is considered on the next frame boundary.
fn next_writer_command(
    control_rx: &Receiver<WriterCommand>,
    data_rx: &Receiver<WriterCommand>,
) -> Option<WriterCommand> {
    loop {
        let control_disconnected = match control_rx.try_recv() {
            Ok(command) => return Some(command),
            Err(TryRecvError::Disconnected) => true,
            Err(TryRecvError::Empty) => false,
        };
        let data_disconnected = match data_rx.try_recv() {
            Ok(command) => return Some(command),
            Err(TryRecvError::Disconnected) => true,
            Err(TryRecvError::Empty) => false,
        };
        if control_disconnected && data_disconnected {
            return None;
        }

        if !control_disconnected {
            match control_rx.recv_timeout(Duration::from_millis(2)) {
                Ok(command) => return Some(command),
                Err(RecvTimeoutError::Disconnected | RecvTimeoutError::Timeout) => {}
            }
        }
        if !data_disconnected {
            match data_rx.recv_timeout(Duration::from_millis(2)) {
                Ok(command) => return Some(command),
                Err(RecvTimeoutError::Disconnected | RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

fn spawn_stderr_drain<R>(mut stderr: R) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let _ = io::copy(&mut stderr, &mut io::sink());
    })
}
