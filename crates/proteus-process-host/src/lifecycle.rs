use std::{
    fmt,
    process::{Child, ExitStatus},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};

const CHILD_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Portable terminal status for one child-process generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessExit {
    code: Option<i32>,
    success: bool,
}

impl ProcessExit {
    fn from_status(status: ExitStatus) -> Self {
        Self {
            code: status.code(),
            success: status.success(),
        }
    }

    pub fn code(&self) -> Option<i32> {
        self.code
    }

    pub fn success(&self) -> bool {
        self.success
    }
}

impl fmt::Display for ProcessExit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.success, self.code) {
            (true, Some(code)) => write!(formatter, "successfully with code {code}"),
            (true, None) => formatter.write_str("successfully"),
            (false, Some(code)) => write!(formatter, "with code {code}"),
            (false, None) => formatter.write_str("without an exit code"),
        }
    }
}

#[derive(Clone, Debug)]
enum LifecycleTerminal {
    Exited(ProcessExit),
    Failed(String),
}

impl LifecycleTerminal {
    fn into_result(self) -> Result<ProcessExit> {
        match self {
            Self::Exited(exit) => Ok(exit),
            Self::Failed(reason) => Err(anyhow!(reason)),
        }
    }
}

#[derive(Debug, Default)]
struct ExitSignal {
    terminal: Mutex<Option<LifecycleTerminal>>,
    changed: Condvar,
}

impl ExitSignal {
    fn record(&self, terminal: LifecycleTerminal) {
        let mut current = self
            .terminal
            .lock()
            .expect("process lifecycle mutex poisoned");
        if current.is_none() {
            *current = Some(terminal);
            self.changed.notify_all();
        }
    }

    fn current(&self) -> Option<LifecycleTerminal> {
        self.terminal
            .lock()
            .expect("process lifecycle mutex poisoned")
            .clone()
    }

    fn wait(&self) -> LifecycleTerminal {
        let mut current = self
            .terminal
            .lock()
            .expect("process lifecycle mutex poisoned");
        loop {
            if let Some(terminal) = current.clone() {
                return terminal;
            }
            current = self
                .changed
                .wait(current)
                .expect("process lifecycle mutex poisoned while waiting");
        }
    }

    fn wait_timeout(&self, timeout: Duration) -> Option<LifecycleTerminal> {
        let started = Instant::now();
        let mut current = self
            .terminal
            .lock()
            .expect("process lifecycle mutex poisoned");
        loop {
            if let Some(terminal) = current.clone() {
                return Some(terminal);
            }
            let remaining = timeout.checked_sub(started.elapsed())?;
            if remaining.is_zero() {
                return None;
            }
            let (next, wait) = self
                .changed
                .wait_timeout(current, remaining)
                .expect("process lifecycle mutex poisoned while waiting");
            current = next;
            if wait.timed_out() && current.is_none() {
                return None;
            }
        }
    }
}

#[derive(Debug)]
enum LifecycleCommand {
    Terminate,
}

#[derive(Debug)]
struct LifecycleInner {
    pid: u32,
    commands: mpsc::Sender<LifecycleCommand>,
    exit: Arc<ExitSignal>,
}

/// Cloneable lifecycle handle for one concrete child-process generation.
///
/// Child ownership stays in a dedicated monitor thread. This lets another
/// thread terminate the process while the transport reader or writer is
/// blocked, and exposes exit as a signal independent from the frame queue.
#[derive(Clone)]
pub struct ProcessLifecycle {
    inner: Arc<LifecycleInner>,
}

impl fmt::Debug for ProcessLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessLifecycle")
            .field("pid", &self.pid())
            .field("exit", &self.inner.exit.current())
            .finish()
    }
}

impl ProcessLifecycle {
    pub(crate) fn spawn(child: Child) -> (Self, JoinHandle<()>) {
        let pid = child.id();
        let (command_tx, command_rx) = mpsc::channel();
        let exit = Arc::new(ExitSignal::default());
        let monitor_exit = Arc::clone(&exit);
        let monitor = thread::spawn(move || monitor_child(child, command_rx, monitor_exit));
        let lifecycle = Self {
            inner: Arc::new(LifecycleInner {
                pid,
                commands: command_tx,
                exit,
            }),
        };
        (lifecycle, monitor)
    }

    pub fn pid(&self) -> u32 {
        self.inner.pid
    }

    /// Returns the terminal status if the child monitor has observed it.
    pub fn try_exit(&self) -> Result<Option<ProcessExit>> {
        self.inner
            .exit
            .current()
            .map(LifecycleTerminal::into_result)
            .transpose()
    }

    /// Waits for the child to exit without consulting the frame channel.
    pub fn wait_for_exit(&self, timeout: Duration) -> Result<Option<ProcessExit>> {
        self.inner
            .exit
            .wait_timeout(timeout)
            .map(LifecycleTerminal::into_result)
            .transpose()
    }

    /// Terminates this exact generation and waits until its monitor records a
    /// terminal status. Repeated and concurrent calls are idempotent.
    pub fn terminate(&self) -> Result<ProcessExit> {
        if let Some(exit) = self.try_exit()? {
            return Ok(exit);
        }
        let _ = self.inner.commands.send(LifecycleCommand::Terminate);
        self.inner.exit.wait().into_result()
    }

    pub(crate) fn same_generation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub(crate) fn stopped_reason(&self) -> Option<String> {
        self.inner.exit.current().map(|terminal| match terminal {
            LifecycleTerminal::Exited(exit) => {
                format!("child process {} exited {exit}", self.pid())
            }
            LifecycleTerminal::Failed(reason) => reason,
        })
    }
}

fn monitor_child(
    mut child: Child,
    commands: mpsc::Receiver<LifecycleCommand>,
    exit: Arc<ExitSignal>,
) {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit.record(LifecycleTerminal::Exited(ProcessExit::from_status(status)));
                return;
            }
            Ok(None) => {}
            Err(error) => {
                let terminal = match terminate_child(&mut child) {
                    Ok(status) => LifecycleTerminal::Exited(ProcessExit::from_status(status)),
                    Err(terminate_error) => LifecycleTerminal::Failed(format!(
                        "failed to inspect child process: {error}; termination also failed: {terminate_error}"
                    )),
                };
                exit.record(terminal);
                return;
            }
        }

        match commands.recv_timeout(CHILD_STATUS_POLL_INTERVAL) {
            Ok(LifecycleCommand::Terminate) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let terminal = match terminate_child(&mut child) {
                    Ok(status) => LifecycleTerminal::Exited(ProcessExit::from_status(status)),
                    Err(error) => LifecycleTerminal::Failed(format!(
                        "failed to terminate child process {}: {error}",
                        child.id()
                    )),
                };
                exit.record(terminal);
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn terminate_child(child: &mut Child) -> Result<ExitStatus> {
    if let Some(status) = child.try_wait()? {
        return Ok(status);
    }
    if let Err(kill_error) = child.kill() {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        return Err(kill_error.into());
    }
    Ok(child.wait()?)
}
