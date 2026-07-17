use std::{
    error::Error,
    fmt,
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Result, bail};
use serde_json::Value;

pub(crate) fn compact_json_len(value: &Value) -> Result<usize> {
    #[derive(Default)]
    struct ByteCounter {
        bytes: usize,
    }

    impl Write for ByteCounter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes = self.bytes.checked_add(buffer.len()).ok_or_else(|| {
                io::Error::new(io::ErrorKind::FileTooLarge, "JSON frame length overflow")
            })?;
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.bytes)
}

/// Default maximum number of frames retained inside one process session.
pub const DEFAULT_MAX_BUFFERED_FRAMES: usize = 256;

/// Default maximum compact-JSON size retained inside one process session.
pub const DEFAULT_MAX_BUFFERED_BYTES: usize = 32 * 1024 * 1024;

/// Bounds for frames waiting inside the host.
///
/// The budget covers both the stdout reader queue and JSON-RPC notifications
/// retained by [`crate::ProcessSession`]. Frames already returned to the caller
/// are no longer part of this budget. Per-frame wire limits remain a framing
/// concern (`NewlineJsonFraming` / `ContentLengthFraming`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveLimits {
    max_buffered_frames: usize,
    max_buffered_bytes: usize,
}

impl ReceiveLimits {
    pub const fn new(max_buffered_frames: usize, max_buffered_bytes: usize) -> Self {
        Self {
            max_buffered_frames,
            max_buffered_bytes,
        }
    }

    pub const fn max_buffered_frames(self) -> usize {
        self.max_buffered_frames
    }

    pub const fn max_buffered_bytes(self) -> usize {
        self.max_buffered_bytes
    }

    pub(crate) fn validate(self) -> Result<()> {
        if self.max_buffered_frames == 0 {
            bail!("receive limit max_buffered_frames must be greater than zero");
        }
        if self.max_buffered_bytes == 0 {
            bail!("receive limit max_buffered_bytes must be greater than zero");
        }
        Ok(())
    }
}

impl Default for ReceiveLimits {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_BUFFERED_FRAMES, DEFAULT_MAX_BUFFERED_BYTES)
    }
}

/// A protocol-neutral failure while receiving a raw frame.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReceiveFrameError {
    /// No frame arrived before the caller-provided deadline.
    Timeout { timeout: Duration },
    /// The stdout reader stopped because the stream closed, framing failed, or
    /// the bounded receive budget was exhausted.
    ReaderStopped { reason: String },
}

impl ReceiveFrameError {
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }
}

impl fmt::Display for ReceiveFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { timeout } => write!(
                formatter,
                "child did not send a frame within {}ms",
                timeout.as_millis()
            ),
            Self::ReaderStopped { reason } => formatter.write_str(reason),
        }
    }
}

impl Error for ReceiveFrameError {}

#[derive(Debug)]
pub(crate) struct BufferedFrame {
    value: Value,
    _permit: ReceivePermit,
}

impl BufferedFrame {
    pub(crate) fn new(value: Value, permit: ReceivePermit) -> Self {
        Self {
            value,
            _permit: permit,
        }
    }

    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    pub(crate) fn into_value(self) -> Value {
        self.value
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ReceiveBudget {
    limits: ReceiveLimits,
    state: Arc<Mutex<ReceiveBudgetState>>,
}

#[derive(Debug, Default)]
struct ReceiveBudgetState {
    frames: usize,
    bytes: usize,
}

impl ReceiveBudget {
    pub(crate) fn new(limits: ReceiveLimits) -> Self {
        Self {
            limits,
            state: Arc::new(Mutex::new(ReceiveBudgetState::default())),
        }
    }

    pub(crate) fn reserve(&self, bytes: usize) -> Result<ReceivePermit, String> {
        let mut state = self.state.lock().expect("receive budget mutex poisoned");
        let next_frames = state.frames.saturating_add(1);
        if next_frames > self.limits.max_buffered_frames {
            return Err(format!(
                "receive buffer exceeded frame count limit: attempted {next_frames}, max {}",
                self.limits.max_buffered_frames
            ));
        }

        let next_bytes = state.bytes.saturating_add(bytes);
        if next_bytes > self.limits.max_buffered_bytes {
            return Err(format!(
                "receive buffer exceeded aggregate byte limit: attempted {next_bytes}, max {}",
                self.limits.max_buffered_bytes
            ));
        }

        state.frames = next_frames;
        state.bytes = next_bytes;
        Ok(ReceivePermit {
            budget: self.clone(),
            bytes,
        })
    }
}

#[derive(Debug)]
pub(crate) struct ReceivePermit {
    budget: ReceiveBudget,
    bytes: usize,
}

impl Drop for ReceivePermit {
    fn drop(&mut self) {
        let mut state = self
            .budget
            .state
            .lock()
            .expect("receive budget mutex poisoned");
        debug_assert!(state.frames > 0);
        debug_assert!(state.bytes >= self.bytes);
        state.frames = state.frames.saturating_sub(1);
        state.bytes = state.bytes.saturating_sub(self.bytes);
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ReaderStatus {
    failure: Arc<Mutex<Option<ReceiveFrameError>>>,
}

impl ReaderStatus {
    pub(crate) fn fail(&self, reason: impl Into<String>) {
        let mut failure = self.failure.lock().expect("reader status mutex poisoned");
        if failure.is_none() {
            *failure = Some(ReceiveFrameError::ReaderStopped {
                reason: reason.into(),
            });
        }
    }

    pub(crate) fn failure_if_any(&self) -> Option<ReceiveFrameError> {
        self.failure
            .lock()
            .expect("reader status mutex poisoned")
            .clone()
    }

    pub(crate) fn failure(&self) -> ReceiveFrameError {
        self.failure_if_any()
            .unwrap_or_else(|| ReceiveFrameError::ReaderStopped {
                reason: "child stdout reader stopped".to_owned(),
            })
    }
}
