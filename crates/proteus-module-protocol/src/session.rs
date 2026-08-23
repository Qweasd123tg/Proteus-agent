use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use proteus_contracts::contracts::{
    PROCESS_MODULE_ACTIVITY_METHOD, PROCESS_MODULE_CANCEL_METHOD, PROCESS_MODULE_CANCELLED_CODE,
    PROCESS_MODULE_PROGRESS_METHOD, ProcessComponentCall, ProcessComponentExportRef,
    ProcessModuleCancel,
};
use proteus_process_host::{
    NewlineJsonFraming, ProcessHost, ProcessSession, ProcessSpec, ReceiveFrameError, ReceiveLimits,
};
use serde_json::{Value, json};

use crate::{
    HostRequestDispatcher, NoHostRequests, ProcessComponentBinding, ProcessContractAuthority,
    ProcessModuleHostRequest, ProcessModuleInvocationResult, ProcessModuleNotification,
    ProcessModuleRpcError, ProcessModuleTerminal,
    envelope::{self, IncomingMessage},
    handshake::initialize_session,
};

pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_CANCEL_GRACE: Duration = Duration::from_millis(250);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessComponentSessionOptions {
    pub handshake_timeout: Duration,
    pub cancel_grace: Duration,
    pub receive_limits: ReceiveLimits,
    pub notification_limits: ReceiveLimits,
}

impl Default for ProcessComponentSessionOptions {
    fn default() -> Self {
        Self {
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
            cancel_grace: DEFAULT_CANCEL_GRACE,
            receive_limits: ReceiveLimits::default(),
            notification_limits: ReceiveLimits::default(),
        }
    }
}

/// Persistent strict-v2 session for one explicitly bound process component.
/// Callback authority is resolved from the active export on every invocation.
pub struct ProcessComponentSession {
    binding: ProcessComponentBinding,
    host: ProcessHost<NewlineJsonFraming>,
    dispatcher: Arc<dyn HostRequestDispatcher>,
    next_invocation_id: AtomicU64,
    cancel_grace: Duration,
    notification_limits: ReceiveLimits,
}

impl std::fmt::Debug for ProcessComponentSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessComponentSession")
            .field("binding", &self.binding)
            .field("host", &self.host)
            .field("cancel_grace", &self.cancel_grace)
            .field("notification_limits", &self.notification_limits)
            .finish_non_exhaustive()
    }
}

impl ProcessComponentSession {
    pub fn connect(
        spec: ProcessSpec,
        binding: ProcessComponentBinding,
        options: ProcessComponentSessionOptions,
    ) -> Result<Self> {
        Self::connect_with_dispatcher(spec, binding, options, Arc::new(NoHostRequests))
    }

    pub fn connect_with_dispatcher(
        spec: ProcessSpec,
        binding: ProcessComponentBinding,
        options: ProcessComponentSessionOptions,
        dispatcher: Arc<dyn HostRequestDispatcher>,
    ) -> Result<Self> {
        validate_options(options)?;
        let initialize = binding.initialize()?;
        let handshake_binding = binding.clone();
        let handshake_timeout = options.handshake_timeout;
        let host =
            ProcessHost::with_initializer(spec, NewlineJsonFraming::default(), move |session| {
                initialize_session(session, &initialize, &handshake_binding, handshake_timeout)
            })
            .receive_limits(options.receive_limits);

        let connected = Self {
            binding,
            host,
            dispatcher,
            next_invocation_id: AtomicU64::new(1),
            cancel_grace: options.cancel_grace,
            notification_limits: options.notification_limits,
        };
        connected.ensure_initialized()?;
        Ok(connected)
    }

    pub fn binding(&self) -> &ProcessComponentBinding {
        &self.binding
    }

    pub fn authority(
        &self,
        target: &ProcessComponentExportRef,
    ) -> Result<ProcessContractAuthority> {
        Ok(*self.binding.export(target)?.authority()?)
    }

    pub fn ensure_initialized(&self) -> Result<()> {
        drop(self.host.ensure_session().with_context(|| {
            format!(
                "process component {:?} handshake failed",
                self.binding.component_id
            )
        })?);
        Ok(())
    }

    pub fn invoke(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<ProcessModuleInvocationResult> {
        self.invoke_with_cancel_check(target, method, params, timeout, || false)
    }

    pub fn invoke_with_cancel_check(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<ProcessModuleInvocationResult> {
        self.invoke_inner(
            target,
            method,
            params,
            timeout,
            Arc::clone(&self.dispatcher),
            is_cancelled,
        )
    }

    /// Invokes the module with turn-scoped callback state.
    ///
    /// Persistent sessions can serve multiple turns, but a callback-heavy
    /// contract must never keep one turn's host context in the session. The
    /// dispatcher is therefore bound to this invocation only.
    pub fn invoke_with_dispatcher_and_cancel_check(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
        dispatcher: Arc<dyn HostRequestDispatcher>,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<ProcessModuleInvocationResult> {
        self.invoke_inner(target, method, params, timeout, dispatcher, is_cancelled)
    }

    fn invoke_inner(
        &self,
        target: &ProcessComponentExportRef,
        method: &str,
        params: Value,
        timeout: Duration,
        dispatcher: Arc<dyn HostRequestDispatcher>,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<ProcessModuleInvocationResult> {
        let export = self.binding.export(target)?;
        let authority = *export.authority()?;
        if method.trim().is_empty() {
            bail!("process module method must not be empty");
        }
        if !authority.allows_module_method(method) {
            bail!(
                "module method {method:?} is not part of {}/{}",
                authority.slot,
                authority.contract_version
            );
        }
        if timeout.is_zero() {
            bail!("process module invocation timeout must be greater than zero");
        }

        let invocation_id = format!(
            "invocation-{}",
            self.next_invocation_id.fetch_add(1, Ordering::Relaxed)
        );
        if is_cancelled() {
            return Ok(ProcessModuleInvocationResult {
                invocation_id,
                terminal: ProcessModuleTerminal::Canceled,
                notifications: Vec::new(),
            });
        }

        let run = (|| -> Result<InvocationRun> {
            let mut session = self.host.ensure_session().with_context(|| {
                format!(
                    "process component {:?} export {}/{} failed to start",
                    self.binding.component_id, target.slot, target.module_id
                )
            })?;
            let call = ProcessComponentCall::new(target.clone(), params);
            session.send_frame(envelope::request(
                json!(invocation_id),
                method,
                serde_json::to_value(call)?,
            ))?;
            self.wait_for_invocation(
                &mut session,
                invocation_id.clone(),
                timeout,
                authority,
                target,
                dispatcher.as_ref(),
                &is_cancelled,
            )
        })();

        match run {
            Ok(run) => {
                if run.reset_session {
                    self.host.reset();
                }
                Ok(run.result)
            }
            Err(error) => {
                self.host.reset();
                Err(error)
            }
        }
    }

    pub fn reset(&self) {
        self.host.reset();
    }

    pub fn terminate(&self) -> Result<()> {
        self.host.terminate()
    }

    // Wire v2 threads one sequential invocation scope through this wait loop;
    // P3 removes the whole facade instead of refactoring a legacy surface.
    #[allow(clippy::too_many_arguments)]
    fn wait_for_invocation(
        &self,
        session: &mut ProcessSession<NewlineJsonFraming>,
        invocation_id: String,
        timeout: Duration,
        authority: ProcessContractAuthority,
        target: &ProcessComponentExportRef,
        dispatcher: &dyn HostRequestDispatcher,
        is_cancelled: &impl Fn() -> bool,
    ) -> Result<InvocationRun> {
        let started = Instant::now();
        let mut notifications = Vec::new();
        let mut notification_bytes = 0usize;
        loop {
            if is_cancelled() {
                return Ok(self.cancel_invocation(
                    session,
                    invocation_id,
                    ProcessModuleTerminal::Canceled,
                    notifications,
                    notification_bytes,
                    authority,
                    target,
                    dispatcher,
                ));
            }

            let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                return Ok(self.cancel_invocation(
                    session,
                    invocation_id,
                    ProcessModuleTerminal::TimedOut,
                    notifications,
                    notification_bytes,
                    authority,
                    target,
                    dispatcher,
                ));
            };
            if remaining.is_zero() {
                return Ok(self.cancel_invocation(
                    session,
                    invocation_id,
                    ProcessModuleTerminal::TimedOut,
                    notifications,
                    notification_bytes,
                    authority,
                    target,
                    dispatcher,
                ));
            }

            let wait = remaining.min(CANCELLATION_POLL_INTERVAL);
            let frame = match session.recv_frame(wait) {
                Ok(frame) => frame,
                Err(ReceiveFrameError::Timeout { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            if let Some(terminal) = self.handle_frame(
                session,
                frame,
                &invocation_id,
                &mut notifications,
                &mut notification_bytes,
                authority,
                target,
                dispatcher,
            )? {
                let reset_session = matches!(terminal, ProcessModuleTerminal::Canceled);
                return Ok(InvocationRun {
                    result: ProcessModuleInvocationResult {
                        invocation_id,
                        terminal,
                        notifications,
                    },
                    reset_session,
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_frame(
        &self,
        session: &mut ProcessSession<NewlineJsonFraming>,
        frame: Value,
        invocation_id: &str,
        notifications: &mut Vec<ProcessModuleNotification>,
        notification_bytes: &mut usize,
        authority: ProcessContractAuthority,
        target: &ProcessComponentExportRef,
        dispatcher: &dyn HostRequestDispatcher,
    ) -> Result<Option<ProcessModuleTerminal>> {
        match envelope::parse(frame)? {
            IncomingMessage::Response { id, result } => {
                if id != json!(invocation_id) {
                    bail!(
                        "process module response id {id} did not match active invocation {invocation_id:?}"
                    );
                }
                Ok(Some(match result {
                    Ok(value) => ProcessModuleTerminal::Success(value),
                    Err(error) if error.code == PROCESS_MODULE_CANCELLED_CODE => {
                        ProcessModuleTerminal::Canceled
                    }
                    Err(error) => ProcessModuleTerminal::ModuleError(error),
                }))
            }
            IncomingMessage::Request { id, method, params } => {
                self.dispatch_host_request(
                    session,
                    id,
                    method,
                    params,
                    invocation_id,
                    authority,
                    target,
                    dispatcher,
                )?;
                Ok(None)
            }
            IncomingMessage::Notification { method, params } => {
                if !allowed_module_notification(&method) {
                    bail!("process module sent unsupported notification {method:?}");
                }
                retain_notification(
                    notifications,
                    notification_bytes,
                    ProcessModuleNotification { method, params },
                    self.notification_limits,
                )?;
                Ok(None)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_host_request(
        &self,
        session: &mut ProcessSession<NewlineJsonFraming>,
        id: Value,
        method: String,
        params: Value,
        invocation_id: &str,
        authority: ProcessContractAuthority,
        target: &ProcessComponentExportRef,
        dispatcher: &dyn HostRequestDispatcher,
    ) -> Result<()> {
        if !authority.allows_host_method(&method) {
            let error = ProcessModuleRpcError::new(
                -32601,
                format!(
                    "host method {method:?} is forbidden for {}/{}",
                    authority.slot, authority.contract_version
                ),
            );
            session.send_frame(envelope::error_response(id, &error)?)?;
            bail!(
                "process component {:?} export {}/{} requested forbidden host method {method:?}",
                self.binding.component_id,
                target.slot,
                target.module_id
            );
        }

        let request = ProcessModuleHostRequest {
            invocation_id: invocation_id.to_owned(),
            method,
            params,
        };
        match dispatcher.dispatch(request) {
            Ok(value) => session.send_frame(envelope::response(id, value)),
            Err(error) => session.send_frame(envelope::error_response(id, &error)?),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn cancel_invocation(
        &self,
        session: &mut ProcessSession<NewlineJsonFraming>,
        invocation_id: String,
        cause: ProcessModuleTerminal,
        mut notifications: Vec<ProcessModuleNotification>,
        mut notification_bytes: usize,
        authority: ProcessContractAuthority,
        target: &ProcessComponentExportRef,
        dispatcher: &dyn HostRequestDispatcher,
    ) -> InvocationRun {
        let cancel = serde_json::to_value(ProcessModuleCancel::new(invocation_id.clone()))
            .expect("ProcessModuleCancel serialization cannot fail");
        if session
            .send_frame(envelope::notification(PROCESS_MODULE_CANCEL_METHOD, cancel))
            .is_ok()
        {
            let started = Instant::now();
            while let Some(remaining) = self.cancel_grace.checked_sub(started.elapsed()) {
                if remaining.is_zero() {
                    break;
                }
                let wait = remaining.min(CANCELLATION_POLL_INTERVAL);
                let frame = match session.recv_frame(wait) {
                    Ok(frame) => frame,
                    Err(ReceiveFrameError::Timeout { .. }) => continue,
                    Err(_) => break,
                };
                match self.handle_frame(
                    session,
                    frame,
                    &invocation_id,
                    &mut notifications,
                    &mut notification_bytes,
                    authority,
                    target,
                    dispatcher,
                ) {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => {}
                }
            }
        }

        InvocationRun {
            result: ProcessModuleInvocationResult {
                invocation_id,
                terminal: cause,
                notifications,
            },
            reset_session: true,
        }
    }
}

struct InvocationRun {
    result: ProcessModuleInvocationResult,
    reset_session: bool,
}

fn allowed_module_notification(method: &str) -> bool {
    matches!(
        method,
        PROCESS_MODULE_PROGRESS_METHOD | PROCESS_MODULE_ACTIVITY_METHOD
    )
}

fn retain_notification(
    notifications: &mut Vec<ProcessModuleNotification>,
    retained_bytes: &mut usize,
    notification: ProcessModuleNotification,
    limits: ReceiveLimits,
) -> Result<()> {
    let next_frames = notifications.len().saturating_add(1);
    if next_frames > limits.max_buffered_frames() {
        bail!(
            "process module invocation notifications exceeded frame count limit: attempted {next_frames}, max {}",
            limits.max_buffered_frames()
        );
    }
    let frame_bytes = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "method": &notification.method,
        "params": &notification.params,
    }))?
    .len();
    let next_bytes = retained_bytes.saturating_add(frame_bytes);
    if next_bytes > limits.max_buffered_bytes() {
        bail!(
            "process module invocation notifications exceeded byte limit: attempted {next_bytes}, max {}",
            limits.max_buffered_bytes()
        );
    }
    *retained_bytes = next_bytes;
    notifications.push(notification);
    Ok(())
}

fn validate_options(options: ProcessComponentSessionOptions) -> Result<()> {
    if options.handshake_timeout.is_zero() {
        bail!("process module handshake timeout must be greater than zero");
    }
    if options.cancel_grace.is_zero() {
        bail!("process module cancel grace must be greater than zero");
    }
    validate_limits("receive", options.receive_limits)?;
    validate_limits("notification", options.notification_limits)?;
    Ok(())
}

fn validate_limits(label: &str, limits: ReceiveLimits) -> Result<()> {
    if limits.max_buffered_frames() == 0 {
        bail!("process module {label} frame limit must be greater than zero");
    }
    if limits.max_buffered_bytes() == 0 {
        bail!("process module {label} byte limit must be greater than zero");
    }
    Ok(())
}
