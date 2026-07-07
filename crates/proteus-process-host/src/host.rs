use std::{
    ops::{Deref, DerefMut},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use anyhow::Result;
use serde_json::Value;

use crate::{Framing, ProcessSession, ProcessSpec};

/// Protocol handshake executed on a freshly spawned session before first use.
pub type SessionInitializer<F> = dyn Fn(&mut ProcessSession<F>) -> Result<()> + Send + Sync;

/// Lazy-starting process host that drops failed sessions for restart on next use.
pub struct ProcessHost<F: Framing> {
    spec: ProcessSpec,
    framing: F,
    initializer: Option<Box<SessionInitializer<F>>>,
    session: Mutex<Option<ProcessSession<F>>>,
}

impl<F: Framing> std::fmt::Debug for ProcessHost<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHost")
            .field("spec", &self.spec)
            .field("has_initializer", &self.initializer.is_some())
            .finish_non_exhaustive()
    }
}

impl<F: Framing> ProcessHost<F> {
    pub fn new(spec: ProcessSpec, framing: F) -> Self {
        Self {
            spec,
            framing,
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
            initializer: Some(Box::new(initializer)),
            session: Mutex::new(None),
        }
    }

    pub fn ensure_session(&self) -> Result<ProcessSessionGuard<'_, F>> {
        let mut guard = self.session.lock().expect("process host mutex poisoned");
        if guard.is_none() {
            let mut session = ProcessSession::spawn(&self.spec, self.framing.clone())?;
            if let Some(initializer) = &self.initializer {
                initializer(&mut session)?;
            }
            *guard = Some(session);
        }
        Ok(ProcessSessionGuard { guard })
    }

    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let result = {
            let mut session = self.ensure_session()?;
            session.request(method, params, timeout)
        };
        if result.is_err() {
            self.reset_session();
        }
        result
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        let result = {
            let mut session = self.ensure_session()?;
            session.notify(method, params)
        };
        if result.is_err() {
            self.reset_session();
        }
        result
    }

    pub fn wait_notification(&self, method: &str, timeout: Duration) -> Result<Value> {
        let result = {
            let mut session = self.ensure_session()?;
            session.wait_notification(method, timeout)
        };
        if result.is_err() {
            self.reset_session();
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

    pub fn reset_session(&self) {
        let mut guard = self.session.lock().expect("process host mutex poisoned");
        *guard = None;
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
