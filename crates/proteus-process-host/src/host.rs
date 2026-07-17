use std::{
    ops::{Deref, DerefMut},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use anyhow::Result;
use serde_json::Value;

use crate::{Framing, ProcessSession, ProcessSpec, ReceiveLimits};

/// Protocol handshake executed on a freshly spawned session before first use.
pub type SessionInitializer<F> = dyn Fn(&mut ProcessSession<F>) -> Result<()> + Send + Sync;

/// Lazy-starting process host that drops failed sessions for restart on next use.
pub struct ProcessHost<F: Framing> {
    spec: ProcessSpec,
    framing: F,
    receive_limits: ReceiveLimits,
    initializer: Option<Box<SessionInitializer<F>>>,
    session: Mutex<Option<ProcessSession<F>>>,
}

impl<F: Framing> std::fmt::Debug for ProcessHost<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHost")
            .field("spec", &self.spec)
            .field("receive_limits", &self.receive_limits)
            .field("has_initializer", &self.initializer.is_some())
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
        }
    }

    /// Like [`ProcessHost::new`], but runs `initializer` on every freshly
    /// spawned session (first start and each lazy restart) before the session
    /// serves traffic. Initialization failure discards the session.
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
        if guard.is_none() {
            let mut session = ProcessSession::spawn_with_receive_limits(
                &self.spec,
                self.framing.clone(),
                self.receive_limits,
            )?;
            if let Some(initializer) = &self.initializer {
                initializer(&mut session)?;
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
        let result = {
            let mut session = self.ensure_session()?;
            session.request(method, params, timeout)
        };
        if result.is_err() {
            self.reset();
        }
        result
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        let result = {
            let mut session = self.ensure_session()?;
            session.notify(method, params)
        };
        if result.is_err() {
            self.reset();
        }
        result
    }

    pub fn wait_notification(&self, method: &str, timeout: Duration) -> Result<Value> {
        let result = {
            let mut session = self.ensure_session()?;
            session.wait_notification(method, timeout)
        };
        if result.is_err() {
            self.reset();
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

    /// Terminates and discards the current session. A later operation starts a
    /// fresh child and reruns the initializer.
    pub fn terminate(&self) -> Result<()> {
        let session = self
            .session
            .lock()
            .expect("process host mutex poisoned")
            .take();
        match session {
            Some(mut session) => session.terminate(),
            None => Ok(()),
        }
    }

    /// Discards the current session with best-effort termination. A later
    /// operation lazily starts a fresh child.
    pub fn reset(&self) {
        let session = self
            .session
            .lock()
            .expect("process host mutex poisoned")
            .take();
        drop(session);
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
