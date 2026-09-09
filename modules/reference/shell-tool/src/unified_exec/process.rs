//! PTY and closed-stdin pipe backends behind the same session lifetime contract.
use std::{
    io::{self, Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use proteus_contracts::domain::EXEC_SHELL;

use super::session::{
    ExecSession, ExecSessionOwner, NEXT_SESSION_ID, ProcessControl, ensure_session_janitor, lock,
    prune_session_if_needed, sessions,
};
use crate::sandbox::{EXEC_COMMAND_ENV, SandboxKind, bwrap_args};

pub(super) fn spawn_session(
    command: &str,
    tty: bool,
    workspace: &str,
    workdir: &str,
    sandbox: Option<SandboxKind>,
    owner: ExecSessionOwner,
) -> Result<(i64, Arc<ExecSession>)> {
    ensure_session_janitor()?;
    let (program, args) = match sandbox.as_ref() {
        Some(sandbox @ SandboxKind::Bwrap(_)) => (
            sandbox.executable().display().to_string(),
            bwrap_args(command, workspace, workdir),
        ),
        None => (EXEC_SHELL.to_owned(), vec!["-lc".into(), command.into()]),
    };
    let session = if tty {
        spawn_pty(&program, args, workdir, sandbox, owner)?
    } else {
        spawn_pipes(&program, args, workdir, sandbox, owner)?
    };
    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let mut sessions = lock(sessions());
    prune_session_if_needed(&mut sessions);
    sessions.insert(session_id, Arc::clone(&session));
    Ok((session_id, session))
}

struct PtyControl {
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    // Keep the controlling terminal alive until the session is released.
    _master: Mutex<Box<dyn MasterPty + Send>>,
}

impl ProcessControl for PtyControl {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = lock(&self.writer);
        writer.write_all(bytes)?;
        writer.flush()
    }

    fn interrupt(&self) -> io::Result<()> {
        self.write(b"\x03")
    }

    fn kill(&self) {
        let _ = lock(&self.killer).kill();
    }
}

fn spawn_pty(
    program: &str,
    args: Vec<String>,
    workdir: &str,
    sandbox: Option<SandboxKind>,
    owner: ExecSessionOwner,
) -> Result<Arc<ExecSession>> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| anyhow!("failed to open PTY: {error}"))?;
    // Acquire I/O before spawning so an acquisition failure cannot orphan a child.
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| anyhow!("failed to clone PTY reader: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| anyhow!("failed to take PTY writer: {error}"))?;
    let mut builder = CommandBuilder::new(program);
    builder.args(args);
    builder.cwd(workdir);
    for (key, value) in EXEC_COMMAND_ENV {
        builder.env(key, value);
    }
    let mut child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| anyhow!("failed to spawn command in PTY: {error}"))?;
    let control = PtyControl {
        writer: Mutex::new(writer),
        killer: Mutex::new(child.clone_killer()),
        _master: Mutex::new(pair.master),
    };
    drop(pair.slave);
    let session = Arc::new(ExecSession::new(Box::new(control), true, sandbox, owner));
    read_output(reader, session.clone());
    let wait_session = session.clone();
    std::thread::spawn(move || {
        let code = child.wait().ok().map(|status| status.exit_code() as i32);
        wait_session.mark_exited(code);
    });
    Ok(session)
}

struct PipeControl {
    // The waiter uses try_wait under this same lock. A signal never targets a
    // reaped/reused PID: inspection and signalling happen before releasing it.
    child: Arc<Mutex<Child>>,
}

impl PipeControl {
    #[cfg(unix)]
    fn signal(&self, signal: i32) -> io::Result<()> {
        let mut child = lock(&self.child);
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        let pgid = i32::try_from(child.id()).map_err(io::Error::other)?;
        // SAFETY: Command::process_group(0) created this dedicated group. The
        // unreaped leader keeps its PID reserved for this child generation.
        if unsafe { libc::kill(-pgid, signal) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }
}

impl ProcessControl for PipeControl {
    fn write(&self, _bytes: &[u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "session stdin is closed",
        ))
    }

    fn interrupt(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.signal(libc::SIGINT)
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "process interrupt is not supported by this process backend",
            ))
        }
    }

    fn kill(&self) {
        #[cfg(unix)]
        let _ = self.signal(libc::SIGKILL);
        #[cfg(not(unix))]
        let _ = lock(&self.child).kill();
    }
}

fn spawn_pipes(
    program: &str,
    args: Vec<String>,
    workdir: &str,
    sandbox: Option<SandboxKind>,
    owner: ExecSessionOwner,
) -> Result<Arc<ExecSession>> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in EXEC_COMMAND_ENV {
        command.env(key, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .context("failed to spawn command with pipes")?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    #[cfg(unix)]
    if let Err(error) = set_nonblocking(&stdout).and_then(|()| set_nonblocking(&stderr)) {
        let control = PipeControl {
            child: Arc::new(Mutex::new(child)),
        };
        control.kill();
        let _ = lock(&control.child).wait();
        return Err(error.into());
    }
    let child = Arc::new(Mutex::new(child));
    let control = PipeControl {
        child: child.clone(),
    };
    let session = Arc::new(ExecSession::new(Box::new(control), false, sandbox, owner));
    read_output(stdout, session.clone());
    read_output(stderr, session.clone());
    let wait_session = session.clone();
    std::thread::spawn(move || {
        loop {
            match lock(&child).try_wait() {
                Ok(Some(status)) => {
                    wait_session.mark_exited(Some(exit_code(status)));
                    break;
                }
                Err(_) => {
                    wait_session.mark_exited(None);
                    break;
                }
                Ok(None) => {}
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    Ok(session)
}

#[cfg(unix)]
fn set_nonblocking(file: &impl std::os::fd::AsRawFd) -> io::Result<()> {
    let fd = file.as_raw_fd();
    // SAFETY: the owned child pipe is alive and this only changes file flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn read_output(mut reader: impl Read + Send + 'static, session: Arc<ExecSession>) {
    std::thread::spawn(move || {
        let mut buffer = [0; 8192];
        loop {
            if session.drain_expired() {
                break;
            }
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => session.push_output(&buffer[..size]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
        session.mark_closed();
    });
}

fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}
