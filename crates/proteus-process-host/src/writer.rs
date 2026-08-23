use std::{
    error::Error,
    fmt, io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde_json::Value;

use crate::{Framing, ProcessLifecycle, receive::compact_json_len};

/// Default number of data frames that may wait for the dedicated stdin writer.
pub const DEFAULT_MAX_QUEUED_WRITES: usize = 256;
/// Default reserved capacity for protocol control frames.
pub const DEFAULT_MAX_QUEUED_CONTROL_WRITES: usize = 64;
/// Default aggregate compact-JSON budget for queued data frames.
pub const DEFAULT_MAX_QUEUED_WRITE_BYTES: usize = 32 * 1024 * 1024;
/// Default aggregate compact-JSON budget reserved for queued control frames.
pub const DEFAULT_MAX_QUEUED_CONTROL_WRITE_BYTES: usize = 8 * 1024 * 1024;

const WRITER_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug)]
pub(crate) struct WriterLimits {
    pub(crate) max_queued_control_writes: usize,
    pub(crate) max_queued_writes: usize,
    pub(crate) max_frame_bytes: usize,
    pub(crate) max_queued_control_write_bytes: usize,
    pub(crate) max_queued_write_bytes: usize,
}

/// A protocol-neutral failure while writing a frame.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SendFrameError {
    QueueFull {
        lane: ProcessFrameLane,
        max_queued_writes: usize,
    },
    QueueBytesFull {
        lane: ProcessFrameLane,
        attempted_bytes: usize,
        max_queued_bytes: usize,
    },
    FrameTooLarge {
        bytes: usize,
        max_frame_bytes: usize,
    },
    InvalidFrame {
        reason: String,
    },
    CanceledBeforeWrite,
    WriterStopped {
        reason: String,
    },
}

impl fmt::Display for SendFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull {
                lane,
                max_queued_writes,
            } => write!(
                formatter,
                "process {lane} writer queue is full: max {max_queued_writes} frames"
            ),
            Self::QueueBytesFull {
                lane,
                attempted_bytes,
                max_queued_bytes,
            } => write!(
                formatter,
                "process {lane} writer queue needs {attempted_bytes} bytes, max {max_queued_bytes}"
            ),
            Self::FrameTooLarge {
                bytes,
                max_frame_bytes,
            } => write!(
                formatter,
                "outbound process frame is {bytes} bytes, max {max_frame_bytes}"
            ),
            Self::InvalidFrame { reason } => {
                write!(
                    formatter,
                    "failed to measure outbound process frame: {reason}"
                )
            }
            Self::CanceledBeforeWrite => {
                formatter.write_str("queued frame was canceled before write")
            }
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

    fn failure_if_any(&self) -> Option<SendFrameError> {
        self.failure
            .lock()
            .expect("writer status mutex poisoned")
            .clone()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessFrameLane {
    Control,
    Data,
}

impl fmt::Display for ProcessFrameLane {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Control => formatter.write_str("control"),
            Self::Data => formatter.write_str("data"),
        }
    }
}

const DISPATCH_QUEUED: u8 = 0;
const DISPATCH_WRITING: u8 = 1;
const DISPATCH_WRITTEN: u8 = 2;
const DISPATCH_CANCELED: u8 = 3;
const DISPATCH_FAILED: u8 = 4;

#[derive(Debug)]
struct DispatchCompletion {
    result: Mutex<Option<std::result::Result<(), SendFrameError>>>,
    ready: Condvar,
}

/// Completion token for one frame accepted into a bounded writer lane.
///
/// A queued data frame can be withdrawn before the writer begins it. This lets
/// multiplexed protocol layers settle an invocation canceled during admission
/// without sending a control frame for work the child never observed.
#[derive(Clone, Debug)]
pub struct FrameDispatch {
    state: Arc<AtomicU8>,
    completion: Arc<DispatchCompletion>,
    status: WriterStatus,
    lifecycle: ProcessLifecycle,
}

impl FrameDispatch {
    fn queued(status: WriterStatus, lifecycle: ProcessLifecycle) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(DISPATCH_QUEUED)),
            completion: Arc::new(DispatchCompletion {
                result: Mutex::new(None),
                ready: Condvar::new(),
            }),
            status,
            lifecycle,
        }
    }

    /// Withdraws a frame that has not reached the writer yet.
    pub fn cancel_before_write(&self) -> bool {
        if self
            .state
            .compare_exchange(
                DISPATCH_QUEUED,
                DISPATCH_CANCELED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }
        self.complete(Err(SendFrameError::CanceledBeforeWrite));
        true
    }

    pub fn is_written(&self) -> bool {
        self.state.load(Ordering::Acquire) == DISPATCH_WRITTEN
    }

    /// Returns true once the writer has claimed the frame. A later control
    /// frame cannot overtake it after this point.
    pub fn is_started(&self) -> bool {
        matches!(
            self.state.load(Ordering::Acquire),
            DISPATCH_WRITING | DISPATCH_WRITTEN | DISPATCH_FAILED
        )
    }

    pub fn wait(&self) -> std::result::Result<(), SendFrameError> {
        let mut result = self
            .completion
            .result
            .lock()
            .expect("frame dispatch mutex poisoned");
        loop {
            if let Some(result) = result.clone() {
                return result;
            }
            if let Some(reason) = self.lifecycle.stopped_reason() {
                return Err(self.status.fail(reason));
            }
            if let Some(failure) = self.status.failure_if_any() {
                return Err(failure);
            }
            let (next, _) = self
                .completion
                .ready
                .wait_timeout(result, WRITER_POLL_INTERVAL)
                .expect("frame dispatch mutex poisoned while waiting");
            result = next;
        }
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

    fn finish(&self, result: std::result::Result<(), SendFrameError>) {
        self.state.store(
            if result.is_ok() {
                DISPATCH_WRITTEN
            } else {
                DISPATCH_FAILED
            },
            Ordering::Release,
        );
        self.complete(result);
    }

    fn complete(&self, result: std::result::Result<(), SendFrameError>) {
        let mut current = self
            .completion
            .result
            .lock()
            .expect("frame dispatch mutex poisoned");
        if current.is_none() {
            *current = Some(result);
            self.completion.ready.notify_all();
        }
    }
}

enum WriterCommand {
    Frame {
        value: Value,
        dispatch: FrameDispatch,
        _permit: WriterPermit,
    },
}

#[derive(Debug)]
struct WriterBudget {
    max_bytes: usize,
    queued_bytes: Mutex<usize>,
}

impl WriterBudget {
    fn reserve(
        self: &Arc<Self>,
        lane: ProcessFrameLane,
        bytes: usize,
    ) -> std::result::Result<WriterPermit, SendFrameError> {
        let mut queued = self
            .queued_bytes
            .lock()
            .expect("writer byte budget mutex poisoned");
        let attempted_bytes = queued.saturating_add(bytes);
        if attempted_bytes > self.max_bytes {
            return Err(SendFrameError::QueueBytesFull {
                lane,
                attempted_bytes,
                max_queued_bytes: self.max_bytes,
            });
        }
        *queued = attempted_bytes;
        Ok(WriterPermit {
            bytes,
            budget: Arc::clone(self),
        })
    }

    fn release(&self, bytes: usize) {
        let mut queued = self
            .queued_bytes
            .lock()
            .expect("writer byte budget mutex poisoned");
        *queued = queued.saturating_sub(bytes);
    }
}

#[derive(Debug)]
struct WriterPermit {
    bytes: usize,
    budget: Arc<WriterBudget>,
}

impl Drop for WriterPermit {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

/// Cloneable bounded handle for writing whole frames to one process
/// generation. A dedicated writer thread is the sole owner of child stdin, so
/// concurrent callers cannot interleave bytes from different frames.
#[derive(Clone)]
pub struct ProcessFrameWriter {
    control_commands: SyncSender<WriterCommand>,
    data_commands: SyncSender<WriterCommand>,
    status: WriterStatus,
    stopping: Arc<AtomicBool>,
    lifecycle: ProcessLifecycle,
    max_queued_writes: usize,
    max_queued_control_writes: usize,
    max_frame_bytes: usize,
    data_budget: Arc<WriterBudget>,
    control_budget: Arc<WriterBudget>,
}

impl fmt::Debug for ProcessFrameWriter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessFrameWriter")
            .field("pid", &self.lifecycle.pid())
            .field("max_queued_writes", &self.max_queued_writes)
            .field("max_queued_control_writes", &self.max_queued_control_writes)
            .field("max_frame_bytes", &self.max_frame_bytes)
            .field("stopping", &self.stopping.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl ProcessFrameWriter {
    pub fn send_frame(&self, value: Value) -> std::result::Result<(), SendFrameError> {
        self.queue_frame(value)?.wait()
    }

    pub fn send_control_frame(&self, value: Value) -> std::result::Result<(), SendFrameError> {
        self.queue_control_frame(value)?.wait()
    }

    pub fn queue_frame(&self, value: Value) -> std::result::Result<FrameDispatch, SendFrameError> {
        self.queue(value, ProcessFrameLane::Data)
    }

    pub fn queue_control_frame(
        &self,
        value: Value,
    ) -> std::result::Result<FrameDispatch, SendFrameError> {
        self.queue(value, ProcessFrameLane::Control)
    }

    fn queue(
        &self,
        value: Value,
        lane: ProcessFrameLane,
    ) -> std::result::Result<FrameDispatch, SendFrameError> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(self.status.failure());
        }
        if let Some(reason) = self.lifecycle.stopped_reason() {
            return Err(self.status.fail(reason));
        }

        let bytes = compact_json_len(&value).map_err(|error| SendFrameError::InvalidFrame {
            reason: error.to_string(),
        })?;
        if bytes > self.max_frame_bytes {
            return Err(SendFrameError::FrameTooLarge {
                bytes,
                max_frame_bytes: self.max_frame_bytes,
            });
        }
        let budget = match lane {
            ProcessFrameLane::Control => &self.control_budget,
            ProcessFrameLane::Data => &self.data_budget,
        };
        let permit = budget.reserve(lane, bytes)?;

        let dispatch = FrameDispatch::queued(self.status.clone(), self.lifecycle.clone());
        let command = WriterCommand::Frame {
            value,
            dispatch: dispatch.clone(),
            _permit: permit,
        };
        let (sender, max_queued_writes) = match lane {
            ProcessFrameLane::Control => (&self.control_commands, self.max_queued_control_writes),
            ProcessFrameLane::Data => (&self.data_commands, self.max_queued_writes),
        };
        match sender.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(SendFrameError::QueueFull {
                    lane,
                    max_queued_writes,
                });
            }
            Err(TrySendError::Disconnected(_)) => return Err(self.status.failure()),
        }

        Ok(dispatch)
    }
}

pub(crate) fn spawn_writer<W, F>(
    mut stdin: W,
    framing: F,
    limits: WriterLimits,
    stopping: Arc<AtomicBool>,
    lifecycle: ProcessLifecycle,
) -> (ProcessFrameWriter, JoinHandle<()>)
where
    W: io::Write + Send + 'static,
    F: Framing,
{
    let (control_tx, control_commands) = mpsc::sync_channel(limits.max_queued_control_writes);
    let (data_tx, data_commands) = mpsc::sync_channel(limits.max_queued_writes);
    let status = WriterStatus::default();
    let control_budget = Arc::new(WriterBudget {
        max_bytes: limits.max_queued_control_write_bytes,
        queued_bytes: Mutex::new(0),
    });
    let data_budget = Arc::new(WriterBudget {
        max_bytes: limits.max_queued_write_bytes,
        queued_bytes: Mutex::new(0),
    });
    let writer = ProcessFrameWriter {
        control_commands: control_tx,
        data_commands: data_tx,
        status: status.clone(),
        stopping: Arc::clone(&stopping),
        lifecycle,
        max_queued_writes: limits.max_queued_writes,
        max_queued_control_writes: limits.max_queued_control_writes,
        max_frame_bytes: limits.max_frame_bytes,
        data_budget,
        control_budget,
    };
    let thread = thread::spawn(move || {
        while !stopping.load(Ordering::Acquire) {
            let command = match control_commands.try_recv() {
                Ok(command) => command,
                Err(TryRecvError::Empty) => {
                    match data_commands.recv_timeout(WRITER_POLL_INTERVAL) {
                        Ok(command) => command,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    match data_commands.recv_timeout(WRITER_POLL_INTERVAL) {
                        Ok(command) => command,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            };
            match command {
                WriterCommand::Frame {
                    value,
                    dispatch,
                    _permit,
                } => {
                    if !dispatch.begin_write() {
                        continue;
                    }
                    if stopping.load(Ordering::Acquire) {
                        dispatch.finish(Err(status.fail("process transport stopped")));
                        break;
                    }
                    match framing.write_frame(&mut stdin, &value) {
                        Ok(()) => dispatch.finish(Ok(())),
                        Err(error) => {
                            let failure = status
                                .fail(format!("failed to write frame to child stdin: {error}"));
                            dispatch.finish(Err(failure));
                            break;
                        }
                    }
                }
            }
        }
        stopping.store(true, Ordering::Release);
        let failure = status.fail("child stdin writer stopped");
        for command in control_commands.try_iter().chain(data_commands.try_iter()) {
            let WriterCommand::Frame { dispatch, .. } = command;
            dispatch.finish(Err(failure.clone()));
        }
    });
    (writer, thread)
}
