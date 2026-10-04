//! Bounded output collection and one-shot child process lifecycle.

use std::{
    io::Read,
    process::{Child, ExitStatus},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use super::{HEAD_LIMIT_BYTES, TAIL_LIMIT_BYTES};

/// Head+tail буфер с жёстким потолком памяти: первые `HEAD_LIMIT_BYTES` и
/// последние `TAIL_LIMIT_BYTES` байта, середина дропается.
pub(super) struct BoundedBuffer {
    pub(super) head: Vec<u8>,
    pub(super) tail: std::collections::VecDeque<u8>,
    pub(super) original_len: usize,
}

impl BoundedBuffer {
    pub(super) fn new() -> Self {
        Self {
            head: Vec::new(),
            tail: std::collections::VecDeque::new(),
            original_len: 0,
        }
    }

    pub(super) fn from_bytes(bytes: &[u8]) -> Self {
        let mut buffer = Self::new();
        buffer.push(bytes);
        buffer
    }

    pub(super) fn push(&mut self, data: &[u8]) {
        self.original_len += data.len();
        let mut rest = data;
        if self.head.len() < HEAD_LIMIT_BYTES {
            let take = (HEAD_LIMIT_BYTES - self.head.len()).min(rest.len());
            self.head.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
        }
        if rest.is_empty() {
            return;
        }
        self.tail.extend(rest.iter().copied());
        if self.tail.len() > TAIL_LIMIT_BYTES {
            let excess = self.tail.len() - TAIL_LIMIT_BYTES;
            self.tail.drain(..excess);
        }
    }

    pub(super) fn truncated(&self) -> bool {
        self.original_len > self.head.len() + self.tail.len()
    }

    pub(super) fn to_text(&self) -> String {
        let tail_bytes: Vec<u8> = self.tail.iter().copied().collect();
        if !self.truncated() {
            let mut bytes = self.head.clone();
            bytes.extend_from_slice(&tail_bytes);
            return String::from_utf8_lossy(&bytes).into_owned();
        }
        // A byte limit may split a valid scalar. Trim only the incomplete
        // boundary fragments, keeping genuine malformed bytes lossy-visible.
        let head_end = match std::str::from_utf8(&self.head) {
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            _ => self.head.len(),
        };
        let tail_start = tail_bytes
            .iter()
            .take(3)
            .take_while(|byte| **byte & 0xc0 == 0x80)
            .count();
        let head = String::from_utf8_lossy(&self.head[..head_end]);
        let tail = String::from_utf8_lossy(&tail_bytes[tail_start..]);
        let omitted = self.original_len - head_end - (tail_bytes.len() - tail_start);
        format!(
            "{head}\n{}\n{tail}",
            omitted_marker(omitted, self.original_len)
        )
    }
}

/// Единый формат маркера усечения для терминальных tools (`shell`,
/// `exec_command`/`write_stdin`).
pub(super) fn omitted_marker(omitted: usize, total: usize) -> String {
    format!("[... omitted {omitted} of {total} bytes ...]")
}

pub(super) struct ShellOutput {
    pub(super) status: ExitStatus,
    pub(super) stdout: BoundedBuffer,
    pub(super) stderr: BoundedBuffer,
}

#[cfg(test)]
pub(super) fn wait_with_timeout(
    child: std::process::Child,
    timeout: Duration,
) -> std::io::Result<(ShellOutput, bool)> {
    wait_with_timeout_and_cancel(child, timeout, &mut || Ok(false))
}

pub(super) fn wait_with_timeout_and_cancel(
    mut child: std::process::Child,
    timeout: Duration,
    is_cancelled: &mut dyn FnMut() -> std::io::Result<bool>,
) -> std::io::Result<(ShellOutput, bool)> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = spawn_bounded_reader(stdout);
    let stderr_reader = spawn_bounded_reader(stderr);
    let started = Instant::now();

    let mut exited = None;
    let (status, timed_out) = loop {
        match is_cancelled() {
            Ok(false) => {}
            result => {
                kill_child_tree(&mut child);
                let _ = child.wait();
                let _ = join_reader(stdout_reader);
                let _ = join_reader(stderr_reader);
                return match result {
                    Ok(true) => Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "shell invocation canceled",
                    )),
                    Err(error) => Err(error),
                    Ok(false) => unreachable!(),
                };
            }
        }
        if exited.is_none() {
            #[cfg(unix)]
            {
                exited = crate::child_status::observe_exit(child.id())?;
            }
            #[cfg(not(unix))]
            {
                exited = child.try_wait()?;
            }
        }
        if let Some(status) = exited
            && stdout_reader.is_finished()
            && stderr_reader.is_finished()
        {
            // Reap only after inherited pipes have drained. Until now the
            // process-group number is reserved even if the leader exited.
            let _ = child.wait()?;
            break (status, false);
        }
        if started.elapsed() >= timeout {
            kill_child_tree(&mut child);
            break (
                match exited {
                    Some(status) => {
                        let _ = child.wait()?;
                        status
                    }
                    None => child.wait()?,
                },
                true,
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    Ok((
        ShellOutput {
            status,
            stdout,
            stderr,
        },
        timed_out,
    ))
}

fn spawn_bounded_reader<R>(reader: Option<R>) -> JoinHandle<std::io::Result<BoundedBuffer>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buffer = BoundedBuffer::new();
        let Some(mut reader) = reader else {
            return Ok(buffer);
        };
        let mut buf = [0u8; 8192];
        loop {
            let read = reader.read(&mut buf)?;
            if read == 0 {
                break;
            }
            buffer.push(&buf[..read]);
        }
        Ok(buffer)
    })
}

fn join_reader(
    handle: JoinHandle<std::io::Result<BoundedBuffer>>,
) -> std::io::Result<BoundedBuffer> {
    handle
        .join()
        .map_err(|_| std::io::Error::other("shell output reader thread panicked"))?
}

#[cfg(unix)]
fn kill_child_tree(child: &mut Child) {
    let pgid = child.id() as i32;
    unsafe {
        let _ = libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_child_tree(child: &mut Child) {
    let _ = child.kill();
}

#[cfg(test)]
#[path = "execution/tests.rs"]
mod tests;
