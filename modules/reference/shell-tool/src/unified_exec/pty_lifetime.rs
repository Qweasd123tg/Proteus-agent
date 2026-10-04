//! Own PTY reap and signal authority under one lock.
use super::session::lock;
use portable_pty::Child;
use std::{io, sync::Mutex};

trait PtyChild: Send + Sync {
    fn observe_exit(&mut self) -> io::Result<Option<i32>>;
    fn kill(&mut self);
    fn reap(&mut self) -> io::Result<()>;
}

struct NativeChild(Box<dyn Child + Send + Sync>);

impl PtyChild for NativeChild {
    fn observe_exit(&mut self) -> io::Result<Option<i32>> {
        #[cfg(unix)]
        {
            let pid = self
                .0
                .process_id()
                .ok_or_else(|| io::Error::other("PTY child has no PID"))?;
            crate::child_status::observe_exit(pid)
                .map(|status| status.map(|status| status.code().unwrap_or(1)))
        }
        #[cfg(not(unix))]
        {
            self.0
                .try_wait()
                .map(|status| status.map(|status| status.exit_code() as i32))
        }
    }

    fn kill(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.process_id() {
            // The caller holds lifetime ownership and has not reaped the
            // leader. Its number cannot be recycled while signalling the group.
            unsafe {
                let _ = libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
        let _ = self.0.kill();
    }

    fn reap(&mut self) -> io::Result<()> {
        self.0.wait().map(|_| ())
    }
}

struct LifetimeState {
    child: Box<dyn PtyChild>,
    can_signal: bool,
    finished: bool,
}

pub(super) struct PtyLifetime(Mutex<LifetimeState>);

impl PtyLifetime {
    pub(super) fn new(child: Box<dyn Child + Send + Sync>) -> Self {
        Self::with_child(Box::new(NativeChild(child)))
    }

    fn with_child(child: Box<dyn PtyChild>) -> Self {
        Self(Mutex::new(LifetimeState {
            child,
            can_signal: true,
            finished: false,
        }))
    }

    pub(super) fn observe_exit(&self) -> io::Result<Option<i32>> {
        let mut state = lock(&self.0);
        let result = state.child.observe_exit();
        if result.is_err() {
            // An external reap/lost ownership (ECHILD) must never leave numeric
            // signal targets usable by later store cleanup.
            state.can_signal = false;
        }
        #[cfg(not(unix))]
        if !matches!(result, Ok(None)) {
            // Portable try_wait may reap. Disable signals before unlocking.
            state.can_signal = false;
            state.finished = true;
        }
        result
    }

    pub(super) fn kill(&self) {
        let mut state = lock(&self.0);
        Self::kill_locked(&mut state);
    }

    fn kill_locked(state: &mut LifetimeState) {
        if state.can_signal {
            state.can_signal = false;
            state.child.kill();
        }
    }

    pub(super) fn finish(&self) {
        let mut state = lock(&self.0);
        if state.finished {
            return;
        }
        // Stop descendants while the unreaped leader still reserves its group
        // identity, then permanently revoke signalling before reap.
        Self::kill_locked(&mut state);
        state.finished = true;
        let _ = state.child.reap();
    }
}

#[cfg(all(test, unix))]
#[path = "tests/pty_lifetime.rs"]
mod tests;
