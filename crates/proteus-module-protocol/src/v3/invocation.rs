use std::{
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use proteus_contracts::contracts::ProcessComponentExportRef;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::ProcessModuleRpcError;

use super::{
    broker::ControlCommand,
    notification::{InvocationNotificationReceiver, NotificationSink},
};

const CONTROL_ACK_TIMEOUT: Duration = Duration::from_secs(2);

pub type HostRequestFuture =
    Pin<Box<dyn Future<Output = Result<Value, ProcessModuleRpcError>> + Send + 'static>>;

/// Async, invocation-scoped host capability dispatcher for wire v3.
pub trait AsyncHostRequestDispatcher: Send + Sync + 'static {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture;
}

#[derive(Debug, Default)]
pub struct NoAsyncHostRequests;

impl AsyncHostRequestDispatcher for NoAsyncHostRequests {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        Box::pin(async move {
            Err(ProcessModuleRpcError::new(
                -32601,
                format!("host method is not implemented: {}", request.method),
            ))
        })
    }
}

/// Broker-owned identity and lineage of one active invocation.
///
/// Fields are private so module-supplied data cannot manufacture a parent for
/// nested work. A dispatcher receives this value from the broker and may pass
/// it back to `start_nested_invocation`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationRef {
    pub(crate) id: String,
    pub(crate) generation: u64,
    pub(crate) target: ProcessComponentExportRef,
    pub(crate) root_id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) depth: usize,
    pub(crate) deadline: Instant,
}

impl InvocationRef {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn target(&self) -> &ProcessComponentExportRef {
        &self.target
    }

    pub fn root_id(&self) -> &str {
        &self.root_id
    }

    pub fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

#[derive(Clone, Debug)]
pub struct ComponentHostRequest {
    pub invocation: InvocationRef,
    pub method: String,
    pub params: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelCause {
    User,
    Timeout,
    Shutdown,
}

impl CancelCause {
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Timeout => "timeout",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentFailure {
    ProcessExit,
    Protocol,
    Resource,
    CancelGrace,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InvocationTerminal {
    Success(Value),
    ModuleError(ProcessModuleRpcError),
    Canceled,
    TimedOut,
    ComponentLost(ComponentFailure),
}

impl InvocationTerminal {
    pub(crate) fn canceled(cause: CancelCause) -> Self {
        match cause {
            CancelCause::Timeout => Self::TimedOut,
            CancelCause::User | CancelCause::Shutdown => Self::Canceled,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentBrokerErrorKind {
    InvalidInput,
    Admission,
    ParentInactive,
    RuntimeUnavailable,
    BootstrapClosed,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentBrokerError {
    pub kind: ComponentBrokerErrorKind,
    pub message: String,
}

impl ComponentBrokerError {
    pub(crate) fn new(kind: ComponentBrokerErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn stopped(message: impl Into<String>) -> Self {
        Self::new(ComponentBrokerErrorKind::Stopped, message)
    }
}

impl fmt::Display for ComponentBrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ComponentBrokerError {}

pub(crate) struct StartMeta {
    pub invocation: InvocationRef,
    pub pid: u32,
}

/// Live handle returned after bounded admission. Notifications and terminal
/// completion are independent, so callers may stream one while awaiting the
/// other.
pub struct InvocationHandle {
    invocation: InvocationRef,
    pid: u32,
    control_tx: mpsc::SyncSender<ControlCommand>,
    terminal_rx: Option<oneshot::Receiver<InvocationTerminal>>,
    notifications: Option<InvocationNotificationReceiver>,
    dropped_notifications: Arc<AtomicU64>,
}

impl fmt::Debug for InvocationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationHandle")
            .field("invocation", &self.invocation)
            .field("pid", &self.pid)
            .field(
                "dropped_notifications",
                &self.dropped_notifications.load(Ordering::Acquire),
            )
            .finish_non_exhaustive()
    }
}

impl InvocationHandle {
    pub(crate) fn new(
        meta: StartMeta,
        control_tx: mpsc::SyncSender<ControlCommand>,
        terminal_rx: oneshot::Receiver<InvocationTerminal>,
        notifications: InvocationNotificationReceiver,
        sink: &NotificationSink,
    ) -> Self {
        Self {
            invocation: meta.invocation,
            pid: meta.pid,
            control_tx,
            terminal_rx: Some(terminal_rx),
            notifications: Some(notifications),
            dropped_notifications: sink.dropped_counter(),
        }
    }

    pub fn invocation(&self) -> &InvocationRef {
        &self.invocation
    }

    pub fn id(&self) -> &str {
        self.invocation.id()
    }

    pub fn generation(&self) -> u64 {
        self.invocation.generation()
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Takes the bounded live notification receiver. It is separate from the
    /// terminal receiver, so both may be awaited concurrently.
    pub fn notifications(
        &mut self,
    ) -> Result<InvocationNotificationReceiver, ComponentBrokerError> {
        self.notifications.take().ok_or_else(|| {
            ComponentBrokerError::new(
                ComponentBrokerErrorKind::InvalidInput,
                format!(
                    "invocation {} notification receiver was already taken",
                    self.id()
                ),
            )
        })
    }

    pub fn dropped_notifications(&self) -> u64 {
        self.dropped_notifications.load(Ordering::Acquire)
    }

    pub fn cancel(&self, cause: CancelCause) -> Result<(), ComponentBrokerError> {
        let (ack_tx, ack_rx) = mpsc::channel();
        match self.control_tx.try_send(ControlCommand::Cancel {
            id: self.invocation.id.clone(),
            generation: self.invocation.generation,
            cause,
            ack: ack_tx,
        }) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                return Err(ComponentBrokerError::new(
                    ComponentBrokerErrorKind::Admission,
                    "component broker control queue is full; cancel was not admitted",
                ));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err(ComponentBrokerError::stopped("component broker is stopped"));
            }
        }
        ack_rx.recv_timeout(CONTROL_ACK_TIMEOUT).map_err(|_| {
            ComponentBrokerError::stopped("component broker did not acknowledge cancel")
        })?
    }

    pub async fn result(&mut self) -> Result<InvocationTerminal, ComponentBrokerError> {
        let receiver = self.terminal_rx.take().ok_or_else(|| {
            ComponentBrokerError::new(
                ComponentBrokerErrorKind::InvalidInput,
                format!("invocation {} terminal was already consumed", self.id()),
            )
        })?;
        receiver.await.map_err(|_| {
            ComponentBrokerError::stopped(format!(
                "component broker stopped before invocation {} settled",
                self.id()
            ))
        })
    }
}
