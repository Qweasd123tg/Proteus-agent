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

use std::{
    io::{BufRead, BufReader as StdBufReader},
    path::PathBuf,
    process::{Child as StdChild, Command as StdCommand, Stdio},
};

use agent_contracts::{
    app_protocol::{StdioOutput, StdioRequest},
    domain::PermissionMode,
};
use anyhow::{Context, Result};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::mpsc,
};

const EXTERNAL_TERMINAL_ENV: &str = "AGENT_SHELL_EXTERNAL_TERMINAL";
const EXTERNAL_TERMINAL_DBUS_ADDRESS_ENV: &str = "AGENT_SHELL_EXTERNAL_DBUS_ADDRESS";

pub struct ExternalTerminalSession {
    dbus_daemon: StdChild,
    address: String,
}

impl ExternalTerminalSession {
    pub fn ptyxis() -> Result<Self> {
        let mut dbus_daemon = StdCommand::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1", "--nopidfile"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to start private D-Bus session for Ptyxis tools")?;
        let stdout = dbus_daemon
            .stdout
            .take()
            .context("private D-Bus session did not expose its address")?;
        let mut address = String::new();
        StdBufReader::new(stdout)
            .read_line(&mut address)
            .context("failed to read private D-Bus address for Ptyxis tools")?;
        let address = address.trim().to_owned();
        if address.is_empty() {
            let _ = dbus_daemon.kill();
            anyhow::bail!("private D-Bus session returned an empty Ptyxis address");
        }
        let service_ready = StdCommand::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.gnome.Ptyxis",
                "--object-path",
                "/org/gnome/Ptyxis",
                "--method",
                "org.freedesktop.DBus.Peer.Ping",
            ])
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to activate dedicated Ptyxis service for terminal tools")?;
        if !service_ready.success() {
            let _ = dbus_daemon.kill();
            anyhow::bail!("dedicated Ptyxis service did not start for terminal tools");
        }
        Ok(Self {
            dbus_daemon,
            address,
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for ExternalTerminalSession {
    fn drop(&mut self) {
        let _ = self.dbus_daemon.kill();
        let _ = self.dbus_daemon.wait();
    }
}

pub struct AgentDriver {
    child: Child,
    stdin: ChildStdin,
    pub events: mpsc::UnboundedReceiver<StdioOutput>,
}

#[derive(Clone)]
pub struct DriverConfig {
    /// Путь к бинарю ядра. Если None — ищем соседний `agent`, затем `agent` в `$PATH`.
    pub agent_bin: Option<PathBuf>,
    /// Путь к config-файлу ядра. Передаётся как `--config`.
    pub config_path: Option<PathBuf>,
    /// Рабочая директория, которую ядро должно использовать.
    pub cwd: Option<PathBuf>,
    /// Session directory для resume. Передаётся как `--resume-session`.
    pub resume_session: Option<PathBuf>,
    /// Optional override for core permission mode.
    pub permission_mode: Option<PermissionMode>,
    /// Private D-Bus address used only when launching visible Ptyxis tool tabs.
    pub external_terminal_dbus_address: Option<String>,
}

impl AgentDriver {
    /// Spawn ядра и подготовка каналов.
    pub async fn spawn(cfg: DriverConfig) -> Result<Self> {
        let program = cfg.agent_bin.unwrap_or_else(default_agent_bin);

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
        if let Some(session_dir) = &cfg.resume_session {
            cmd.arg("--resume-session").arg(session_dir);
        }
        if let Some(mode) = cfg.permission_mode {
            cmd.arg("--permission-mode").arg(permission_mode_arg(mode));
        }
        cmd.arg("server").arg("stdio");
        if let Some(address) = &cfg.external_terminal_dbus_address {
            cmd.env(EXTERNAL_TERMINAL_ENV, "ptyxis");
            cmd.env(EXTERNAL_TERMINAL_DBUS_ADDRESS_ENV, address);
        }
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
    pub async fn shutdown(&mut self) -> Result<()> {
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

pub(crate) fn permission_mode_arg(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Plan => "plan",
        PermissionMode::Normal => "normal",
        PermissionMode::Auto => "auto",
        _ => "normal",
    }
}

fn default_agent_bin() -> PathBuf {
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let sibling = dir.join("agent");
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from("agent")
}
