use std::{
    ops::{Deref, DerefMut},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{Framing, ProcessLifecycle, ProcessSession, ProcessSpec, ReceiveLimits};

/// Protocol handshake executed on a freshly spawned session before first use.
pub type SessionInitializer<F> = dyn Fn(&mut ProcessSession<F>) -> Result<()> + Send + Sync;

/// Lazy-starting sequential facade that drops failed generations for restart
/// on next use.
///
/// Session traffic remains single-caller for MCP and LSP. Lifecycle is
/// tracked separately so `terminate`/`reset` can stop a child while that caller
/// is blocked waiting for input.
pub struct ProcessHost<F: Framing> {
    spec: ProcessSpec,
    framing: F,
    receive_limits: ReceiveLimits,
    initializer: Option<Box<SessionInitializer<F>>>,
    session: Mutex<Option<ProcessSession<F>>>,
    active_lifecycle: Mutex<Option<ProcessLifecycle>>,
}

impl<F: Framing> std::fmt::Debug for ProcessHost<F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let active_pid = self
            .active_lifecycle
            .lock()
            .expect("process host lifecycle mutex poisoned")
            .as_ref()
            .map(ProcessLifecycle::pid);
        formatter
            .debug_struct("ProcessHost")
            .field("spec", &self.spec)
            .field("receive_limits", &self.receive_limits)
            .field("has_initializer", &self.initializer.is_some())
            .field("active_pid", &active_pid)
            .finish_non_exhaustive()
    }
}

impl<F: Framing> ProcessHost<F> {
    pub fn new(spec: ProcessSpec, framing: F) -> Self {
        Self {
            spec,
            framing,
            receive_limits: ReceiveLimits::default(),
            initializer: None,
            session: Mutex::new(None),
            active_lifecycle: Mutex::new(None),
        }
    }

    /// Like [`ProcessHost::new`], but runs `initializer` on every freshly
    /// spawned generation (first start and each lazy restart) before it serves
    /// traffic. Initialization failure discards that generation.
    pub fn with_initializer(
        spec: ProcessSpec,
        framing: F,
        initializer: impl Fn(&mut ProcessSession<F>) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            spec,
            framing,
            receive_limits: ReceiveLimits::default(),
            initializer: Some(Box::new(initializer)),
            session: Mutex::new(None),
            active_lifecycle: Mutex::new(None),
        }
    }

    /// Overrides the bounds shared by the stdout queue and retained
    /// JSON-RPC notifications. Invalid zero limits fail on first spawn.
    pub fn receive_limits(mut self, receive_limits: ReceiveLimits) -> Self {
        self.receive_limits = receive_limits;
        self
    }

    pub fn ensure_session(&self) -> Result<ProcessSessionGuard<'_, F>> {
        let mut guard = self.session.lock().expect("process host mutex poisoned");
        self.discard_stopped_session(&mut guard);
        if guard.is_none() {
            let mut session = ProcessSession::spawn_with_receive_limits(
                &self.spec,
                self.framing.clone(),
                self.receive_limits,
            )?;
            let lifecycle = session.lifecycle();
            self.set_active_lifecycle(Some(lifecycle.clone()));

            if let Some(initializer) = &self.initializer
                && let Err(error) = initializer(&mut session)
            {
                self.clear_active_lifecycle(&lifecycle);
                drop(session);
                return Err(error);
            }

            match lifecycle.try_exit() {
                Ok(None) => {}
                Ok(Some(exit)) => {
                    self.clear_active_lifecycle(&lifecycle);
                    drop(session);
                    bail!(
                        "child process {} exited {exit} during session initialization",
                        lifecycle.pid()
                    );
                }
                Err(error) => {
                    self.clear_active_lifecycle(&lifecycle);
                    drop(session);
                    return Err(error)
                        .context("child lifecycle failed during session initialization");
                }
            }
            *guard = Some(session);
        }
        Ok(ProcessSessionGuard { guard })
    }

    /// Sends a raw frame without applying JSON-RPC classification or automatic
    /// session reset.
    pub fn send_frame(&self, message: Value) -> Result<()> {
        self.ensure_session()?.send_frame(message)
    }

    /// Receives a raw frame. Timeout does not terminate or reset the session;
    /// callers can downcast the returned `anyhow::Error` to
    /// [`crate::ReceiveFrameError`] when they need typed timeout handling.
    pub fn recv_frame(&self, timeout: Duration) -> Result<Value> {
        self.ensure_session()?
            .recv_frame(timeout)
            .map_err(Into::into)
    }

    /// Non-blocking raw receive with no protocol classification.
    pub fn try_recv_frame(&self) -> Result<Option<Value>> {
        self.ensure_session()?.try_recv_frame().map_err(Into::into)
    }

    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let mut session = self.ensure_session()?;
        let lifecycle = session.lifecycle();
        let result = session.request(method, params, timeout);
        drop(session);
        if result.is_err() {
            self.reset_generation(&lifecycle);
        }
        result
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        let mut session = self.ensure_session()?;
        let lifecycle = session.lifecycle();
        let result = session.notify(method, params);
        drop(session);
        if result.is_err() {
            self.reset_generation(&lifecycle);
        }
        result
    }

    pub fn wait_notification(&self, method: &str, timeout: Duration) -> Result<Value> {
        let mut session = self.ensure_session()?;
        let lifecycle = session.lifecycle();
        let result = session.wait_notification(method, timeout);
        drop(session);
        if result.is_err() {
            self.reset_generation(&lifecycle);
        }
        result
    }

    pub fn drain_notifications(&self) -> Vec<Value> {
        let mut guard = self.session.lock().expect("process host mutex poisoned");
        guard
            .as_mut()
            .map(ProcessSession::drain_notifications)
            .unwrap_or_default()
    }

    /// Terminates and discards the current generation. A later operation
    /// starts a fresh child and reruns the initializer. The lifecycle signal is
    /// sent without waiting for the sequential session mutex, so a blocked
    /// reader wakes immediately.
    pub fn terminate(&self) -> Result<()> {
        let lifecycle = self.active_lifecycle();
        let result = match &lifecycle {
            Some(lifecycle) => lifecycle.terminate().map(|_| ()),
            None => Ok(()),
        };
        self.discard_generation(lifecycle.as_ref());
        result
    }

    /// Discards the current generation with best-effort termination. A later
    /// operation lazily starts a fresh child.
    pub fn reset(&self) {
        let lifecycle = self.active_lifecycle();
        if let Some(lifecycle) = &lifecycle {
            let _ = lifecycle.terminate();
        }
        self.discard_generation(lifecycle.as_ref());
    }

    fn reset_generation(&self, lifecycle: &ProcessLifecycle) {
        let _ = lifecycle.terminate();
        self.discard_generation(Some(lifecycle));
    }

    fn discard_stopped_session(&self, guard: &mut Option<ProcessSession<F>>) {
        let stopped = guard.as_ref().is_some_and(|session| {
            session
                .lifecycle()
                .try_exit()
                .map_or(true, |exit| exit.is_some())
        });
        if !stopped {
            return;
        }
        if let Some(session) = guard.take() {
            let lifecycle = session.lifecycle();
            self.clear_active_lifecycle(&lifecycle);
            drop(session);
        }
    }

    fn discard_generation(&self, generation: Option<&ProcessLifecycle>) {
        let session = {
            let mut guard = self.session.lock().expect("process host mutex poisoned");
            let matches = match (guard.as_ref(), generation) {
                (Some(session), Some(generation)) => {
                    session.lifecycle().same_generation(generation)
                }
                (Some(_), None) => true,
                (None, _) => false,
            };
            matches.then(|| guard.take()).flatten()
        };
        if let Some(session) = session {
            let lifecycle = session.lifecycle();
            self.clear_active_lifecycle(&lifecycle);
            drop(session);
        } else if let Some(generation) = generation {
            self.clear_active_lifecycle(generation);
        }
    }

    fn active_lifecycle(&self) -> Option<ProcessLifecycle> {
        self.active_lifecycle
            .lock()
            .expect("process host lifecycle mutex poisoned")
            .clone()
    }

    fn set_active_lifecycle(&self, lifecycle: Option<ProcessLifecycle>) {
        *self
            .active_lifecycle
            .lock()
            .expect("process host lifecycle mutex poisoned") = lifecycle;
    }

    fn clear_active_lifecycle(&self, generation: &ProcessLifecycle) {
        let mut active = self
            .active_lifecycle
            .lock()
            .expect("process host lifecycle mutex poisoned");
        if active
            .as_ref()
            .is_some_and(|active| active.same_generation(generation))
        {
            *active = None;
        }
    }
}

pub struct ProcessSessionGuard<'a, F: Framing> {
    guard: MutexGuard<'a, Option<ProcessSession<F>>>,
}

impl<F: Framing> Deref for ProcessSessionGuard<'_, F> {
    type Target = ProcessSession<F>;

    fn deref(&self) -> &Self::Target {
        self.guard
            .as_ref()
            .expect("process host guard must contain a session")
    }
}

impl<F: Framing> DerefMut for ProcessSessionGuard<'_, F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("process host guard must contain a session")
    }
}
