//! Subprocess driver: запускает `agent server stdio` и даёт каналы для
//! чтения событий и отправки команд.
//!
//! Архитектура:
//! - Один `tokio::process::Child` для ядра.
//! - Фоновая task читает stdout child'а построчно, парсит JSON, шлёт
//!   `StdioOutput` в `mpsc::Receiver`.
//! - `send_request` пишет `StdioRequest` в stdin child'а.
//! - Stderr child'а просто форвардится в наш stderr — это ядро логирует
//!   загрузку плагинов и т.п., видно в терминале если TUI запущен из него.

use std::path::PathBuf;

use agent_contracts::app_protocol::{StdioOutput, StdioRequest};
use anyhow::{Context, Result};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::mpsc,
};

pub struct AgentDriver {
    child: Child,
    stdin: ChildStdin,
    pub events: mpsc::UnboundedReceiver<StdioOutput>,
}

pub struct DriverConfig {
    /// Путь к бинарю ядра. Если None — `modular-agent` ищется в `$PATH`.
    pub agent_bin: Option<PathBuf>,
    /// Путь к config-файлу ядра. Передаётся как `--config`.
    pub config_path: Option<PathBuf>,
    /// Рабочая директория, которую ядро должно использовать.
    pub cwd: Option<PathBuf>,
}

impl AgentDriver {
    /// Spawn ядра и подготовка каналов.
    pub async fn spawn(cfg: DriverConfig) -> Result<Self> {
        let program = cfg
            .agent_bin
            .unwrap_or_else(|| PathBuf::from("modular-agent"));

        let mut cmd = Command::new(&program);
        // Важно: --config / --cwd должны идти ДО positional args
        // `server stdio`. Иначе clap видит их как часть `task` и ядро
        // запускает обычный turn с текстом "server stdio --config ...".
        if let Some(config_path) = &cfg.config_path {
            cmd.arg("--config").arg(config_path);
        }
        if let Some(cwd) = &cfg.cwd {
            cmd.arg("--cwd").arg(cwd);
        }
        cmd.arg("server").arg("stdio");
        // stderr ядра → /tmp/agent-tui-core.log (append).
        // Никаких eprintln здесь — мы уже можем быть в alternate screen,
        // и вывод поверх ratatui ломает кадр. Путь к логу фиксирован:
        // используй `tail -f /tmp/agent-tui-core.log` параллельно.
        let log_path = std::env::temp_dir().join("agent-tui-core.log");
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .with_context(|| format!("failed to open log {}", log_path.display()))?;
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::from(log_file))
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn {}", program.display()))?;

        let stdin = child.stdin.take().context("child has no stdin")?;
        let stdout = child.stdout.take().context("child has no stdout")?;

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<StdioOutput>(line) {
                            Ok(output) => {
                                if event_tx.send(output).is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                eprintln!(
                                    "[tui] failed to parse stdio output: {err} — line: {line}"
                                );
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        eprintln!("[tui] error reading child stdout: {err}");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            child,
            stdin,
            events: event_rx,
        })
    }

    /// Пишет запрос в stdin ядра. Каждый запрос — JSONL (одна строка с \n).
    pub async fn send(&mut self, request: &StdioRequest) -> Result<()> {
        let json = serde_json::to_string(request)?;
        self.stdin.write_all(json.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Отправить Shutdown и дождаться exit кода.
    pub async fn shutdown(mut self) -> Result<()> {
        let _ = self.send(&StdioRequest::Shutdown { id: None }).await;
        // Дадим ядру время завершиться gracefully.
        let wait_result =
            tokio::time::timeout(std::time::Duration::from_secs(3), self.child.wait()).await;
        match wait_result {
            Ok(Ok(_status)) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(_) => {
                let _ = self.child.start_kill();
                Ok(())
            }
        }
    }
}
