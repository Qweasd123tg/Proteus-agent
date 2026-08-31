use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use proteus_contracts::contracts::ProcessComponentExportRef;
use proteus_process_host::ProcessSpec;
use serde_json::Value;
use tokio::{runtime::Handle, sync::oneshot};

use crate::{ProcessComponentBinding, ProcessContractAuthority};

use super::{
    config::ComponentBrokerOptions,
    invocation::{
        AsyncHostRequestDispatcher, CancelCause, ComponentBrokerError, ComponentBrokerErrorKind,
        ComponentFailure, InvocationHandle, InvocationRef, InvocationTerminal, NoAsyncHostRequests,
        StartMeta,
    },
    notification::NotificationSink,
    pending::{LoopState, TerminalSender},
};

const COMMAND_ACK_SLACK: Duration = Duration::from_secs(2);

pub(crate) struct StartRequest {
    pub(super) target: ProcessComponentExportRef,
    pub(super) method: String,
    pub(super) params: Value,
    pub(super) deadline: Instant,
    pub(super) parent: Option<InvocationRef>,
    pub(super) dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
    pub(super) executor: Option<Handle>,
    pub(super) terminal: TerminalSender,
    pub(super) notifications: NotificationSink,
    pub(super) ack: StartAck,
    pub(super) bootstrap: bool,
}

pub(super) enum StartAck {
    Async(oneshot::Sender<Result<StartMeta, ComponentBrokerError>>),
    Blocking(mpsc::Sender<Result<StartMeta, ComponentBrokerError>>),
}

impl StartAck {
    pub(super) fn send(self, result: Result<StartMeta, ComponentBrokerError>) {
        match self {
            Self::Async(sender) => {
                let _ = sender.send(result);
            }
            Self::Blocking(sender) => {
                let _ = sender.send(result);
            }
        }
    }
}

pub(crate) enum ControlCommand {
    EnsureInitialized {
        ack: mpsc::Sender<Result<(u64, u32), String>>,
    },
    StartNested(Box<StartRequest>),
    Cancel {
        id: String,
        generation: u64,
        cause: CancelCause,
        ack: mpsc::Sender<Result<(), ComponentBrokerError>>,
    },
    CallbackComplete {
        generation: u64,
        callback_id: String,
        result: Result<Value, crate::ProcessModuleRpcError>,
    },
    Inspect {
        ack: mpsc::Sender<ComponentBrokerSnapshot>,
    },
    Reset {
        ack: mpsc::Sender<()>,
    },
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentBrokerSnapshot {
    pub generation: u64,
    pub pid: Option<u32>,
    pub active_invocations: usize,
    pub pending_invocations: usize,
    pub pending_callbacks: usize,
    pub last_failure: Option<ComponentFailure>,
    pub last_failure_reason: Option<String>,
}

struct BrokerInner {
    root_tx: SyncSender<StartRequest>,
    control_tx: SyncSender<ControlCommand>,
    binding: ProcessComponentBinding,
    options: ComponentBrokerOptions,
    stopped: AtomicBool,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for BrokerInner {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.control_tx.send(ControlCommand::Shutdown);
        if let Some(thread) = self
            .thread
            .lock()
            .expect("component broker thread mutex poisoned")
            .take()
        {
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
pub struct ComponentBroker {
    inner: Arc<BrokerInner>,
}

impl std::fmt::Debug for ComponentBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComponentBroker")
            .field("binding", &self.inner.binding)
            .field("options", &self.inner.options)
            .field("stopped", &self.inner.stopped.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct WeakComponentBroker {
    inner: Weak<BrokerInner>,
}

impl WeakComponentBroker {
    pub fn upgrade(&self) -> Option<ComponentBroker> {
        self.inner.upgrade().map(|inner| ComponentBroker { inner })
    }
}

impl ComponentBroker {
    pub fn connect(
        spec: ProcessSpec,
        binding: ProcessComponentBinding,
        options: ComponentBrokerOptions,
    ) -> Result<Self> {
        options.validate()?;
        let (root_tx, root_rx) = mpsc::sync_channel(options.root_command_capacity);
        let (control_tx, control_rx) = mpsc::sync_channel(options.control_command_capacity);
        let thread_control_tx = control_tx.clone();
        let thread_binding = binding.clone();
        let thread = thread::Builder::new()
            .name(format!("component-v3-{}", binding.component_id))
            .spawn(move || {
                let mut state = LoopState::new(spec, thread_binding, options, thread_control_tx);
                state.run(root_rx, control_rx);
            })?;
        let broker = Self {
            inner: Arc::new(BrokerInner {
                root_tx,
                control_tx,
                binding,
                options,
                stopped: AtomicBool::new(false),
                thread: Mutex::new(Some(thread)),
            }),
        };
        broker.ensure_initialized()?;
        Ok(broker)
    }

    pub fn binding(&self) -> &ProcessComponentBinding {
        &self.inner.binding
    }

    pub fn authority(
        &self,
        target: &ProcessComponentExportRef,
    ) -> Result<ProcessContractAuthority> {
        Ok(*self.inner.binding.export(target)?.authority()?)
    }

    pub fn downgrade(&self) -> WeakComponentBroker {
        WeakComponentBroker {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub fn ensure_initialized(&self) -> Result<()> {
        let (ack_tx, ack_rx) = mpsc::channel();
        self.inner
            .control_tx
            .send(ControlCommand::EnsureInitialized { ack: ack_tx })
            .map_err(|_| anyhow::anyhow!("component broker is stopped"))?;
        ack_rx
            .recv_timeout(
                self.inner
                    .options
                    .handshake_timeout
                    .saturating_add(COMMAND_ACK_SLACK),
            )
            .context("component broker did not complete initialization")?
            .map(|_| ())
            .map_err(anyhow::Error::msg)
    }

    pub async fn start_invocation(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<InvocationHandle, ComponentBrokerError> {
        self.start_invocation_with_dispatcher(
            target,
            method,
            params,
            timeout,
            Arc::new(NoAsyncHostRequests),
        )
        .await
    }

    pub async fn start_invocation_with_dispatcher(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
    ) -> Result<InvocationHandle, ComponentBrokerError> {
        self.start_async(None, target, method, params, timeout, dispatcher)
            .await
    }

    pub async fn start_nested_invocation(
        &self,
        parent: &InvocationRef,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
    ) -> Result<InvocationHandle, ComponentBrokerError> {
        self.start_async(
            Some(parent.clone()),
            target,
            method,
            params,
            timeout,
            dispatcher,
        )
        .await
    }

    async fn start_async(
        &self,
        parent: Option<InvocationRef>,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
    ) -> Result<InvocationHandle, ComponentBrokerError> {
        validate_call(method, timeout)?;
        let executor = Handle::try_current().map_err(|_| {
            ComponentBrokerError::new(
                ComponentBrokerErrorKind::RuntimeUnavailable,
                "component-v3 runtime invocation requires an active Tokio runtime",
            )
        })?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            ComponentBrokerError::new(
                ComponentBrokerErrorKind::InvalidInput,
                "component-v3 invocation deadline overflowed",
            )
        })?;
        let (terminal_tx, terminal_rx) = oneshot::channel();
        let (notifications, notification_rx) =
            NotificationSink::channel(self.inner.options.notification_limits);
        let handle_sink = notifications.clone();
        let (ack_tx, ack_rx) = oneshot::channel();
        let request = StartRequest {
            target: target.clone(),
            method: method.to_owned(),
            params,
            deadline,
            parent: parent.clone(),
            dispatcher,
            executor: Some(executor),
            terminal: TerminalSender::Async(terminal_tx),
            notifications,
            ack: StartAck::Async(ack_tx),
            bootstrap: false,
        };
        if parent.is_some() {
            match self
                .inner
                .control_tx
                .try_send(ControlCommand::StartNested(Box::new(request)))
            {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    return Err(ComponentBrokerError::new(
                        ComponentBrokerErrorKind::Admission,
                        "component-v3 nested admission queue is full",
                    ));
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(ComponentBrokerError::stopped("component broker is stopped"));
                }
            }
        } else {
            match self.inner.root_tx.try_send(request) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    return Err(ComponentBrokerError::new(
                        ComponentBrokerErrorKind::Admission,
                        "component-v3 root admission queue is full",
                    ));
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(ComponentBrokerError::stopped("component broker is stopped"));
                }
            }
        }
        let meta = ack_rx.await.map_err(|_| {
            ComponentBrokerError::stopped("component broker dropped start admission")
        })??;
        Ok(InvocationHandle::new(
            meta,
            self.inner.control_tx.clone(),
            terminal_rx,
            notification_rx,
            &handle_sink,
        ))
    }

    pub async fn invoke(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<InvocationTerminal, ComponentBrokerError> {
        let mut handle = self
            .start_invocation(target, method, params, timeout)
            .await?;
        handle.result().await
    }

    pub async fn invoke_with_dispatcher(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
    ) -> Result<InvocationTerminal, ComponentBrokerError> {
        let mut handle = self
            .start_invocation_with_dispatcher(target, method, params, timeout, dispatcher)
            .await?;
        handle.result().await
    }

    /// Synchronous config-build probe. It is callback-free and is rejected
    /// permanently after the first normal async invocation is admitted.
    pub fn invoke_bootstrap(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<InvocationTerminal, ComponentBrokerError> {
        self.invoke_blocking_inner(None, target, method, params, timeout, true)
    }

    /// Callback-free invocation for a synchronous slot contract. It uses the
    /// same bounded broker and wire routing as async calls, but settles through
    /// a blocking channel because the contract has no async entrypoint.
    pub fn invoke_blocking(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<InvocationTerminal, ComponentBrokerError> {
        self.invoke_blocking_inner(None, target, method, params, timeout, false)
    }

    /// Callback-free synchronous child of an active invocation. This keeps
    /// synchronous policy traits inside the same lineage and cancel
    /// tree when a process callback re-enters another export of this broker.
    pub fn invoke_nested_blocking(
        &self,
        parent: &InvocationRef,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<InvocationTerminal, ComponentBrokerError> {
        self.invoke_blocking_inner(Some(parent.clone()), target, method, params, timeout, false)
    }

    fn invoke_blocking_inner(
        &self,
        parent: Option<InvocationRef>,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
        bootstrap: bool,
    ) -> Result<InvocationTerminal, ComponentBrokerError> {
        validate_call(method, timeout)?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            ComponentBrokerError::new(
                ComponentBrokerErrorKind::InvalidInput,
                "component-v3 blocking invocation deadline overflowed",
            )
        })?;
        let (terminal_tx, terminal_rx) = mpsc::channel();
        let (notifications, _notification_rx) =
            NotificationSink::channel(self.inner.options.notification_limits);
        let (ack_tx, ack_rx) = mpsc::channel();
        let request = StartRequest {
            target: target.clone(),
            method: method.to_owned(),
            params,
            deadline,
            parent: parent.clone(),
            dispatcher: Arc::new(NoAsyncHostRequests),
            executor: None,
            terminal: TerminalSender::Blocking(terminal_tx),
            notifications,
            ack: StartAck::Blocking(ack_tx),
            bootstrap,
        };
        if parent.is_some() {
            self.inner
                .control_tx
                .try_send(ControlCommand::StartNested(Box::new(request)))
                .map_err(|error| {
                    ComponentBrokerError::new(
                        ComponentBrokerErrorKind::Admission,
                        format!("component-v3 blocking nested admission failed: {error}"),
                    )
                })?;
        } else {
            self.inner.root_tx.try_send(request).map_err(|error| {
                ComponentBrokerError::new(
                    ComponentBrokerErrorKind::Admission,
                    format!("component-v3 blocking admission failed: {error}"),
                )
            })?;
        }
        ack_rx
            .recv_timeout(
                self.inner
                    .options
                    .handshake_timeout
                    .saturating_add(COMMAND_ACK_SLACK),
            )
            .map_err(|_| {
                ComponentBrokerError::stopped("component broker did not admit blocking invocation")
            })??;
        terminal_rx
            .recv_timeout(
                timeout
                    .saturating_add(self.inner.options.cancel_grace)
                    .saturating_add(COMMAND_ACK_SLACK),
            )
            .map_err(|_| {
                ComponentBrokerError::stopped("component-v3 blocking invocation did not settle")
            })
    }

    pub fn snapshot(&self) -> Result<ComponentBrokerSnapshot, ComponentBrokerError> {
        let (ack_tx, ack_rx) = mpsc::channel();
        self.inner
            .control_tx
            .send(ControlCommand::Inspect { ack: ack_tx })
            .map_err(|_| ComponentBrokerError::stopped("component broker is stopped"))?;
        ack_rx
            .recv_timeout(COMMAND_ACK_SLACK)
            .map_err(|_| ComponentBrokerError::stopped("component broker did not answer inspect"))
    }

    /// Explicitly discards the current generation. The next invocation starts
    /// a fresh process and repeats the exact handshake.
    pub fn reset(&self) -> Result<(), ComponentBrokerError> {
        let (ack_tx, ack_rx) = mpsc::channel();
        self.inner
            .control_tx
            .send(ControlCommand::Reset { ack: ack_tx })
            .map_err(|_| ComponentBrokerError::stopped("component broker is stopped"))?;
        ack_rx.recv_timeout(COMMAND_ACK_SLACK).map_err(|_| {
            ComponentBrokerError::stopped("component broker did not acknowledge reset")
        })
    }
}

fn validate_call(method: &str, timeout: Duration) -> Result<(), ComponentBrokerError> {
    if method.trim().is_empty() {
        return Err(ComponentBrokerError::new(
            ComponentBrokerErrorKind::InvalidInput,
            "component-v3 method must not be empty",
        ));
    }
    if timeout.is_zero() {
        return Err(ComponentBrokerError::new(
            ComponentBrokerErrorKind::InvalidInput,
            "component-v3 invocation timeout must be greater than zero",
        ));
    }
    Ok(())
}
