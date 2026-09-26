//! Lifecycle дочернего процесса `proteus server stdio`: spawn с piped
//! stdio, ограниченный reader stdout, запись JSONL-запросов
//! в stdin, kill по требованию.
//!
//! Reader-таск декаплит чтение от turn-логики: пока родитель ждёт approval
//! у пользователя, события ребёнка буферизуются до явного лимита. stderr ребёнка
//! уходит в null — диагностика ребёнка живёт в его собственном event log.

use std::{path::Path, process::Stdio};

use anyhow::{Context, Result, anyhow};
use proteus_contracts::app_protocol::{StdioOutput, StdioRequest};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, ChildStdin, Command},
};

use super::output::{ChildOutputs, OutputHealth};

pub(super) struct ChildProcess {
    child: Child,
    stdin: ChildStdin,
    outputs: ChildOutputs,
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

        let outputs = ChildOutputs::spawn(stdout);

        Ok(Self {
            child,
            stdin,
            outputs,
        })
    }

    pub async fn send(&mut self, request: &StdioRequest) -> Result<()> {
        self.outputs.health().check()?;
        let mut line = serde_json::to_string(request).context("serialize child stdio request")?;
        line.push('\n');
        let health = self.outputs.health();
        tokio::select! {
            biased;
            error = health.stopped() => Err(error),
            result = async {
                self.stdin.write_all(line.as_bytes()).await.context("write to subagent child stdin")?;
                self.stdin.flush().await.context("flush subagent child stdin")
            } => result,
        }
    }

    /// Следующий output ребёнка. `None` — stdout закрыт (ребёнок умер).
    pub async fn next_output(&mut self) -> Result<Option<StdioOutput>> {
        self.outputs.next().await
    }

    pub fn output_health(&self) -> OutputHealth {
        self.outputs.health()
    }

    /// Выгребает накопившиеся с прошлого turn-а outputs, не блокируясь.
    pub fn drain_stale_outputs(&mut self) -> Result<()> {
        self.outputs.drain()
    }

    pub fn is_alive(&mut self) -> bool {
        self.outputs.health().is_running() && matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }

    #[cfg(test)]
    pub(super) fn test_fixture() -> Self {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("while read -r _line; do :; done")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn child fixture");
        let stdin = child.stdin.take().expect("fixture stdin");
        let outputs = ChildOutputs::spawn(child.stdout.take().expect("fixture stdout"));
        Self {
            child,
            stdin,
            outputs,
        }
    }
}
