//! Owns an ordinary Proteus HTTP process. No runtime or module wiring lives here.
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use proteus_client_common::{desktop::DesktopConnection, normalize_local_origin};
use serde::Deserialize;

pub struct Backend {
    child: Child,
    pub connection: DesktopConnection,
    log: Arc<Mutex<VecDeque<String>>>,
}

#[derive(Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum ReadyRecord {
    #[serde(rename = "http_ready")]
    HttpReady { origin: String },
}

impl Backend {
    pub fn launch(
        bin_dir: &Path,
        workspace: &Path,
        config: &str,
        stopping: &AtomicBool,
    ) -> Result<Self> {
        if !workspace.is_dir() {
            bail!("Папка проекта не найдена: {}", workspace.display());
        }
        let token = uuid::Uuid::new_v4().simple().to_string();
        let mut paths = vec![bin_dir.to_path_buf()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let mut command = Command::new(bin_dir.join("proteus"));
        command
            .current_dir(workspace)
            .env("PATH", std::env::join_paths(paths)?)
            .args(["--config", config, "--cwd"])
            .arg(workspace)
            .args([
                "server",
                "http",
                "--port",
                "0",
                "--token",
                &token,
                "--ready-stdout",
            ])
            .args([
                "--allow-origin",
                "tauri://localhost",
                "--allow-origin",
                "http://tauri.localhost",
                "--allow-origin",
                "http://127.0.0.1:1430",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .context("Не удалось запустить backend из пакета приложения")?;
        let stdout = child.stdout.take().context("Backend stdout is missing")?;
        let stderr = child.stderr.take().context("Backend stderr is missing")?;
        let log = Arc::new(Mutex::new(VecDeque::new()));
        let (send, receive) = mpsc::channel();
        read_output(stdout, log.clone(), token.clone(), Some(send));
        read_output(stderr, log.clone(), token.clone(), None);
        let mut backend = Self {
            child,
            log,
            connection: DesktopConnection {
                app_server_origin: String::new(),
                token,
                workspace: workspace.display().to_string(),
            },
        };
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if stopping.load(Ordering::Relaxed) {
                bail!("Запуск отменён");
            }
            match receive.recv_timeout(Duration::from_millis(100)) {
                Ok(origin) => {
                    let origin = normalize_local_origin(&origin).map_err(anyhow::Error::msg)?;
                    backend.connection.app_server_origin = origin;
                    // Readiness is followed by a real authenticated request, including CORS.
                    backend.check_connection()?;
                    return Ok(backend);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("Backend закрыл канал запуска.\n{}", backend.diagnostics());
                }
            }
            if let Some(status) = backend.child.try_wait()? {
                bail!(
                    "Backend завершился при запуске ({status}).\n{}",
                    backend.diagnostics()
                );
            }
            if Instant::now() >= deadline {
                bail!(
                    "Backend не запустился за 90 секунд.\n{}",
                    backend.diagnostics()
                );
            }
        }
    }

    fn request(&self, method: &str, path: &str) -> Result<String> {
        let address: SocketAddr = self
            .connection
            .app_server_origin
            .strip_prefix("http://")
            .context("Backend must use HTTP")?
            .parse()?;
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nOrigin: tauri://localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            self.connection.token
        )?;
        let mut response = String::new();
        stream.take(4 * 1024 * 1024).read_to_string(&mut response)?;
        Ok(response)
    }

    fn check_connection(&self) -> Result<()> {
        let response = self
            .request("GET", "/bootstrap")
            .context("Backend запущен, но подключение не удалось")?;
        if !response.starts_with("HTTP/1.1 200 ")
            || !response
                .to_lowercase()
                .contains("access-control-allow-origin: tauri://localhost")
        {
            bail!("Backend отклонил подключение приложения");
        }
        Ok(())
    }

    pub fn exit_error(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!(
                "Backend завершился ({status}).\n{}",
                self.diagnostics()
            )),
            Err(error) => Some(format!("Не удалось проверить backend: {error}")),
            Ok(None) => None,
        }
    }

    pub fn diagnostics(&self) -> String {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            if !self.connection.app_server_origin.is_empty() {
                let _ = self.request("POST", "/shutdown");
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(50));
            }
        }
        // Reap remaining descendants even if the direct child already exited.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        if self.child.try_wait().ok().flatten().is_none() {
            thread::sleep(Duration::from_millis(200));
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn read_output(
    reader: impl Read + Send + 'static,
    log: Arc<Mutex<VecDeque<String>>>,
    token: String,
    ready: Option<mpsc::Sender<String>>,
) {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else {
                break;
            };
            if let (Some(send), Ok(ReadyRecord::HttpReady { origin })) =
                (&ready, serde_json::from_str(&line))
            {
                let _ = send.send(origin);
                continue;
            }
            let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
            if log.len() == 60 {
                log.pop_front();
            }
            log.push_back(
                line.replace(&token, "[session credential]")
                    .chars()
                    .take(2000)
                    .collect(),
            );
        }
    });
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
