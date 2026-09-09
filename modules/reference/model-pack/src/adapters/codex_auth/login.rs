use std::{
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use super::{
    CLIENT_ID,
    oauth::{OAuthClient, Pkce},
    store::Credentials,
};

const CALLBACK_PORT: u16 = 1455;
const REDIRECT: &str = "http://localhost:1455/auth/callback";

pub(super) async fn browser(oauth: &OAuthClient, open_browser: bool) -> Result<Credentials> {
    let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .context("cannot listen on localhost:1455; close another login or use --device-auth")?;
    let pkce = Pkce::new();
    let url = oauth.authorize_url(REDIRECT, &pkce)?;
    println!("Откройте ссылку и войдите в ChatGPT:\n{url}");
    if open_browser {
        launch_browser(url.as_str());
    }
    browser_callback(listener, oauth, &pkce, REDIRECT).await
}

pub(super) async fn browser_callback(
    listener: TcpListener,
    oauth: &OAuthClient,
    pkce: &Pkce,
    redirect: &str,
) -> Result<Credentials> {
    loop {
        let (mut socket, _) = listener.accept().await?;
        let path = match tokio::time::timeout(Duration::from_secs(5), read_path(&mut socket)).await
        {
            Ok(Ok(path)) => path,
            _ => continue,
        };
        let url = reqwest::Url::parse(&format!("http://localhost{path}"))?;
        if url.path() != "/auth/callback" {
            reply(&mut socket, "404 Not Found", "Not found").await?;
            continue;
        }
        let pairs = url.query_pairs().collect::<Vec<_>>();
        let values = |key: &str| {
            pairs
                .iter()
                .filter(|(k, _)| k == key)
                .map(|(_, v)| v.as_ref())
                .collect::<Vec<_>>()
        };
        if values("state") != [pkce.state.as_str()] {
            reply(&mut socket, "400 Bad Request", "Invalid OAuth state").await?;
            continue;
        }
        if !values("error").is_empty() {
            reply(
                &mut socket,
                "400 Bad Request",
                "ChatGPT login was declined. Return to the terminal.",
            )
            .await?;
            bail!("ChatGPT login was declined");
        }
        let codes = values("code");
        if codes.len() != 1 || codes[0].is_empty() {
            reply(&mut socket, "400 Bad Request", "Missing authorization code").await?;
            continue;
        }
        let result = oauth.exchange(codes[0], redirect, &pkce.verifier).await;
        // Do not reflect provider errors or tokens into HTML, stdout or logs.
        let (status, message) = if result.is_ok() {
            (
                "200 OK",
                "ChatGPT authorization received. Return to Proteus to confirm it was saved. You can close this tab.",
            )
        } else {
            (
                "400 Bad Request",
                "ChatGPT login failed. Return to the terminal.",
            )
        };
        // Losing the browser connection after token rotation must not discard credentials.
        let _ = reply(&mut socket, status, message).await;
        return result;
    }
}

async fn read_path(socket: &mut TcpStream) -> Result<String> {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 1024];
        let n = socket.read(&mut chunk).await?;
        if n == 0 || bytes.len() + n > 16 * 1024 {
            bail!("invalid OAuth callback");
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let text = std::str::from_utf8(&bytes)?;
    let mut parts = text
        .lines()
        .next()
        .context("missing HTTP request")?
        .split_whitespace();
    if parts.next() != Some("GET") {
        bail!("expected GET callback");
    }
    let path = parts.next().context("missing callback path")?;
    if !path.starts_with('/') || path.starts_with("//") {
        bail!("invalid callback path");
    }
    Ok(path.to_owned())
}

async fn reply(socket: &mut TcpStream, status: &str, message: &str) -> Result<()> {
    socket.write_all(format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{message}", message.len(),
    ).as_bytes()).await?;
    Ok(())
}

pub(super) async fn device(oauth: &OAuthClient) -> Result<Credentials> {
    #[derive(Deserialize)]
    struct Device {
        device_auth_id: String,
        user_code: String,
        interval: String,
    }
    #[derive(Deserialize)]
    struct Code {
        authorization_code: String,
        code_verifier: String,
    }
    let response = oauth
        .http
        .post(format!("{}/api/accounts/deviceauth/usercode", oauth.issuer))
        .json(&json!({"client_id": CLIENT_ID}))
        .send()
        .await?;
    if !response.status().is_success() {
        bail!(
            "ChatGPT device login returned {}; enable device-code login in ChatGPT settings or use browser login",
            response.status()
        );
    }
    let device: Device = response
        .json()
        .await
        .context("invalid ChatGPT device response")?;
    let interval = device
        .interval
        .parse::<u64>()
        .context("invalid ChatGPT polling interval")?
        .max(1);
    println!(
        "Откройте {}/codex/device\nКод: {}",
        oauth.issuer, device.user_code
    );
    loop {
        let response = oauth
            .http
            .post(format!("{}/api/accounts/deviceauth/token", oauth.issuer))
            .json(&json!({"device_auth_id": device.device_auth_id, "user_code": device.user_code}))
            .send()
            .await?;
        if response.status().is_success() {
            let code: Code = response
                .json()
                .await
                .context("invalid ChatGPT device authorization")?;
            return oauth
                .exchange(
                    &code.authorization_code,
                    &format!("{}/deviceauth/callback", oauth.issuer),
                    &code.code_verifier,
                )
                .await;
        }
        if !matches!(response.status().as_u16(), 403 | 404) {
            bail!("ChatGPT device polling returned {}", response.status());
        }
        tokio::time::sleep(Duration::from_secs(interval.saturating_add(3))).await;
    }
}

fn launch_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = Command::new("rundll32");
        cmd.arg("url.dll,FileProtocolHandler");
        cmd
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = Command::new("xdg-open");
    match command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(_) => eprintln!("Не удалось открыть браузер; откройте напечатанную ссылку вручную."),
    }
}
