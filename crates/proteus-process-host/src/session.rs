use std::{
    io::{self, BufReader, Read},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::{Framing, ProcessSpec};

/// Live persistent child process session.
#[derive(Debug)]
pub struct ProcessSession<F: Framing> {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: Receiver<Result<Value>>,
    next_request_id: i64,
    notifications: Vec<Value>,
    framing: F,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl<F: Framing> ProcessSession<F> {
    pub fn spawn(spec: &ProcessSpec, framing: F) -> Result<Self> {
        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .envs(&spec.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open child stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to open child stdout"))?;
        let stderr = child.stderr.take();

        let (stdout_rx, stdout_thread) = spawn_stdout_reader(stdout, framing.clone());
        let stderr_thread = stderr.map(spawn_stderr_drain);

        Ok(Self {
            child,
            stdin,
            stdout_rx,
            next_request_id: 1,
            notifications: Vec::new(),
            framing,
            stdout_thread: Some(stdout_thread),
            stderr_thread,
        })
    }

    pub fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let request_id = self.next_request_id();
        let request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        self.framing.write_frame(&mut self.stdin, &request)?;
        self.recv_response(request_id, timeout)
    }

    pub fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.framing.write_frame(&mut self.stdin, &notification)
    }

    pub fn drain_notifications(&mut self) -> Vec<Value> {
        self.notifications.drain(..).collect()
    }

    pub fn wait_notification(&mut self, method: &str, timeout: Duration) -> Result<Value> {
        if let Some(index) = self
            .notifications
            .iter()
            .position(|message| notification_method(message) == Some(method))
        {
            return Ok(self.notifications.remove(index));
        }

        let started = Instant::now();
        loop {
            let message = self.recv_frame_before(started, timeout, "notification")?;
            if notification_method(&message) == Some(method) {
                return Ok(message);
            }
            if is_notification(&message) {
                self.notifications.push(message);
            }
        }
    }

    fn recv_response(&mut self, expected_id: i64, timeout: Duration) -> Result<Value> {
        let started = Instant::now();
        loop {
            let message = self.recv_frame_before(started, timeout, "response")?;
            if is_notification(&message) {
                self.notifications.push(message);
                continue;
            }

            let Some(id) = message.get("id") else {
                continue;
            };
            if id != &json!(expected_id) {
                bail!("response id {id} did not match expected id {expected_id}");
            }
            if let Some(error) = message.get("error") {
                bail!("JSON-RPC error response: {error}");
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| anyhow!("JSON-RPC response missing result"));
        }
    }

    fn recv_frame_before(
        &mut self,
        started: Instant,
        timeout: Duration,
        expected: &str,
    ) -> Result<Value> {
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            self.kill_and_wait();
            bail!(
                "child did not send {expected} within {}ms",
                timeout.as_millis()
            );
        }

        match self.stdout_rx.recv_timeout(timeout - elapsed) {
            Ok(value) => value,
            Err(RecvTimeoutError::Timeout) => {
                self.kill_and_wait();
                bail!(
                    "child did not send {expected} within {}ms",
                    timeout.as_millis()
                );
            }
            Err(RecvTimeoutError::Disconnected) => bail!("child stdout reader stopped"),
        }
    }

    fn next_request_id(&mut self) -> i64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        request_id
    }

    fn kill_and_wait(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl<F: Framing> Drop for ProcessSession<F> {
    fn drop(&mut self) {
        self.kill_and_wait();
        if let Some(thread) = self.stdout_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

fn spawn_stdout_reader<R, F>(reader: R, framing: F) -> (Receiver<Result<Value>>, JoinHandle<()>)
where
    R: Read + Send + 'static,
    F: Framing,
{
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        loop {
            let value = framing.read_frame(&mut reader);
            let done = value.is_err();
            if tx.send(value).is_err() || done {
                break;
            }
        }
    });
    (rx, thread)
}

fn spawn_stderr_drain<R>(mut reader: R) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let _ = io::copy(&mut reader, &mut io::sink());
    })
}

fn is_notification(message: &Value) -> bool {
    message.get("id").is_none() && notification_method(message).is_some()
}

fn notification_method(message: &Value) -> Option<&str> {
    message.get("method").and_then(Value::as_str)
}
