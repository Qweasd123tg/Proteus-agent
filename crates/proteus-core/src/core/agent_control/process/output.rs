//! Bounded stdout buffering. Overflow is a peer failure, never silent event
//! loss or a blocked reader hiding control responses behind progress events.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use proteus_contracts::app_protocol::StdioOutput;
use proteus_process_host::{
    DEFAULT_MAX_BUFFERED_BYTES, DEFAULT_MAX_BUFFERED_FRAMES, DEFAULT_MAX_FRAME_BYTES,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, BufReader},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
    task::JoinHandle,
};

struct BufferedOutput {
    output: StdioOutput,
    _bytes: OwnedSemaphorePermit,
}

#[derive(Clone)]
enum ReaderState {
    Running,
    Closed,
    Failed(String),
}

/// Separate from the data queue so failure wakes an outstanding approval or
/// user-input wait even when that queue is full.
#[derive(Clone)]
pub(super) struct OutputHealth(watch::Receiver<ReaderState>);

impl OutputHealth {
    pub(super) fn is_running(&self) -> bool {
        matches!(*self.0.borrow(), ReaderState::Running)
    }

    pub(super) fn check(&self) -> Result<()> {
        if let ReaderState::Failed(message) = &*self.0.borrow() {
            bail!("{message}");
        }
        Ok(())
    }

    pub(super) async fn stopped(&self) -> anyhow::Error {
        let mut state = self.0.clone();
        let _ = state
            .wait_for(|value| !matches!(value, ReaderState::Running))
            .await;
        match &*state.borrow() {
            ReaderState::Failed(message) => anyhow!(message.clone()),
            _ => anyhow!("subagent child stdout closed"),
        }
    }
}

pub(super) struct ChildOutputs {
    queue: mpsc::Receiver<BufferedOutput>,
    health: OutputHealth,
    reader: JoinHandle<()>,
}

impl ChildOutputs {
    pub(super) fn spawn(stdout: impl AsyncRead + Unpin + Send + 'static) -> Self {
        let (sender, queue) = mpsc::channel(DEFAULT_MAX_BUFFERED_FRAMES);
        let (state, health) = watch::channel(ReaderState::Running);
        let reader = tokio::spawn(async move {
            let result = read_outputs(stdout, &sender).await;
            // Publish the failure before dropping the data sender and waking
            // a receiver waiting on an empty queue.
            state.send_replace(match result {
                Ok(()) => ReaderState::Closed,
                Err(error) => ReaderState::Failed(format!("{error:#}")),
            });
        });
        Self {
            queue,
            health: OutputHealth(health),
            reader,
        }
    }

    pub(super) fn health(&self) -> OutputHealth {
        self.health.clone()
    }

    pub(super) async fn next(&mut self) -> Result<Option<StdioOutput>> {
        self.health.check()?;
        let output = self.queue.recv().await;
        // The failed reader drops its sender. Never turn buffered responses
        // into success after an overflow has invalidated the peer stream.
        self.health.check()?;
        Ok(output.map(|buffered| buffered.output))
    }

    pub(super) fn drain(&mut self) -> Result<()> {
        self.health.check()?;
        // Bounded work even if an idle peer keeps producing output.
        for _ in 0..DEFAULT_MAX_BUFFERED_FRAMES {
            if self.queue.try_recv().is_err() {
                break;
            }
        }
        self.health.check()
    }
}

impl Drop for ChildOutputs {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

async fn read_outputs(
    stdout: impl AsyncRead + Unpin,
    sender: &mpsc::Sender<BufferedOutput>,
) -> Result<()> {
    let bytes = Arc::new(Semaphore::new(DEFAULT_MAX_BUFFERED_BYTES));
    let mut reader = BufReader::new(stdout);
    while let Some(line) = read_line(&mut reader).await? {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let permit = bytes
            .clone()
            .try_acquire_many_owned(line.len() as u32)
            .map_err(|_| {
                anyhow!("subagent stdout exceeded {DEFAULT_MAX_BUFFERED_BYTES} buffered bytes")
            })?;
        let output =
            serde_json::from_slice(&line).context("invalid subagent stdout JSONL output")?;
        match sender.try_send(BufferedOutput {
            output,
            _bytes: permit,
        }) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                bail!("subagent stdout exceeded {DEFAULT_MAX_BUFFERED_FRAMES} buffered outputs");
            }
        }
    }
    Ok(())
}

/// Check the wire limit before growing the frame, including unterminated
/// lines. BufReader::lines() could allocate without bound before enqueue.
async fn read_line(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf().await.context("read subagent stdout")?;
        if chunk.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let end = chunk.iter().position(|byte| *byte == b'\n');
        let take = end.map_or(chunk.len(), |end| end + 1);
        if take > DEFAULT_MAX_FRAME_BYTES - line.len() {
            bail!("subagent stdout frame exceeded {DEFAULT_MAX_FRAME_BYTES} bytes");
        }
        line.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if end.is_some() {
            return Ok(Some(line));
        }
    }
}
