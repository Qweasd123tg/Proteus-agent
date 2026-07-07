//! Lifecycle дочернего процесса `proteus server stdio`: spawn с piped
//! stdio, фоновый reader stdout → unbounded channel, запись JSONL-запросов
//! в stdin, kill по требованию.
//!
//! Reader-таск декаплит чтение от turn-логики: пока родитель ждёт approval
//! у пользователя, события ребёнка не забивают OS pipe. stderr ребёнка
//! уходит в null — диагностика ребёнка живёт в его собственном event log.

use std::{path::Path, process::Stdio};

use anyhow::{Context, Result, anyhow};
use proteus_contracts::app_protocol::{StdioOutput, StdioRequest};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::mpsc,
};

pub(super) struct ChildProcess {
    child: Child,
    stdin: ChildStdin,
    outputs: mpsc::UnboundedReceiver<StdioOutput>,
}

impl ChildProcess {
    pub fn spawn(
        binary: &Path,
        config_ref: &str,
        extra_args: &[String],
        cwd: &Path,
    ) -> Result<Self> {
        let mut command = Command::new(binary);
        command
            .arg("--config")
            .arg(config_ref)
            .arg("--cwd")
            .arg(cwd)
            .arg("--new-session")
            .args(extra_args)
            .arg("server")
            .arg("stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to spawn subagent child process {} (config {config_ref})",
                binary.display()
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("subagent child stdin is not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("subagent child stdout is not piped"))?;

        let (output_tx, outputs) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                // Непарсящиеся строки пропускаем: ребёнок мог напечатать
                // диагностику в stdout до/между JSONL-выводами.
                let Ok(output) = serde_json::from_str::<StdioOutput>(&line) else {
                    continue;
                };
                if output_tx.send(output).is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            child,
            stdin,
            outputs,
        })
    }

    pub async fn send(&mut self, request: &StdioRequest) -> Result<()> {
        let mut line = serde_json::to_string(request).context("serialize child stdio request")?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .context("write to subagent child stdin")?;
        self.stdin
            .flush()
            .await
            .context("flush subagent child stdin")?;
        Ok(())
    }

    /// Следующий output ребёнка. `None` — stdout закрыт (ребёнок умер).
    pub async fn next_output(&mut self) -> Option<StdioOutput> {
        self.outputs.recv().await
    }

    /// Выгребает накопившиеся с прошлого turn-а outputs, не блокируясь.
    pub fn drain_stale_outputs(&mut self) {
        while self.outputs.try_recv().is_ok() {}
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}
