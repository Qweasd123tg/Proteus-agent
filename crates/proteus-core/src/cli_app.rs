use std::{
    collections::{HashMap, HashSet},
    io::{self, IsTerminal, Write},
    path::Path,
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use proteus_contracts::{
    app_protocol::{
        AppHistorySummary, AppRememberResult, AppServerEvent, StdioOutput, StdioRequest,
    },
    contracts::{ApprovalCacheScope, UserInputAnswer, UserInputRequest, UserInputResponse},
    domain::{AgentOutput, PermissionMode},
};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
};

/// Product CLI transport. The parent process owns terminal presentation only;
/// all application/session behavior lives in the local app-server child and
/// crosses the canonical JSONL protocol.
pub(crate) struct CliAppClient {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    interactive_terminal: bool,
    next_id: u64,
    pending_control_responses: HashSet<String>,
}

impl CliAppClient {
    pub(crate) async fn launch(
        config_path: Option<&Path>,
        cwd: &Path,
        resume_session: Option<&Path>,
        permission_mode: PermissionMode,
    ) -> Result<Self> {
        let executable = std::env::current_exe().context("resolve current proteus executable")?;
        let mut command = Command::new(executable);
        if let Some(config_path) = config_path {
            command.arg("--config").arg(config_path);
        }
        command.arg("--cwd").arg(cwd);
        if let Some(session_dir) = resume_session {
            command.arg("--resume-session").arg(session_dir);
        } else {
            // The former direct product path started a fresh session unless
            // resume was explicit. Preserve that behavior at the server
            // launch boundary instead of inheriting `server stdio`'s
            // operator-oriented resume-latest default.
            command.arg("--new-session");
        }
        command
            .arg("--permission-mode")
            .arg(permission_mode_arg(permission_mode)?)
            .arg("server")
            .arg("stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let mut child = command.spawn().context("launch local app-server")?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("local app-server stdin is unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("local app-server stdout is unavailable"))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            interactive_terminal: io::stdin().is_terminal() && io::stdout().is_terminal(),
            next_id: 1,
            pending_control_responses: HashSet::new(),
        })
    }

    pub(crate) async fn send(&mut self, text: String) -> Result<AgentOutput> {
        let id = self.request_id("send");
        let value = self
            .request(StdioRequest::Send { id: Some(id), text })
            .await?
            .ok_or_else(|| anyhow!("app-server send response has no output"))?;
        serde_json::from_value(value).context("decode app-server AgentOutput")
    }

    pub(crate) async fn clear_history(&mut self) -> Result<()> {
        let id = self.request_id("clear");
        self.request(StdioRequest::ClearHistory { id: Some(id) })
            .await?;
        Ok(())
    }

    pub(crate) async fn history_summary(&mut self) -> Result<AppHistorySummary> {
        let id = self.request_id("history");
        let value = self
            .request(StdioRequest::HistorySummary { id: Some(id) })
            .await?
            .ok_or_else(|| anyhow!("app-server history response has no output"))?;
        serde_json::from_value(value).context("decode app-server history summary")
    }

    pub(crate) async fn remember(
        &mut self,
        kind: String,
        content: String,
    ) -> Result<AppRememberResult> {
        let id = self.request_id("remember");
        let value = self
            .request(StdioRequest::Remember {
                id: Some(id),
                kind,
                content,
            })
            .await?
            .ok_or_else(|| anyhow!("app-server remember response has no output"))?;
        serde_json::from_value(value).context("decode app-server remember result")
    }

    pub(crate) async fn config_summary(&mut self) -> Result<Value> {
        let id = self.request_id("config");
        self.request(StdioRequest::ConfigSummary { id: Some(id) })
            .await?
            .ok_or_else(|| anyhow!("app-server config response has no output"))
    }

    pub(crate) async fn shutdown(mut self) -> Result<()> {
        let id = self.request_id("shutdown");
        let response = self.request(StdioRequest::Shutdown { id: Some(id) }).await;
        let _ = self.stdin.shutdown().await;

        let status = match tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await {
            Ok(status) => status.context("wait for local app-server")?,
            Err(_) => {
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
                bail!("local app-server did not stop after shutdown")
            }
        };
        response?;
        if !status.success() {
            bail!("local app-server exited with {status}");
        }
        Ok(())
    }

    async fn request(&mut self, request: StdioRequest) -> Result<Option<Value>> {
        let target_id = request
            .id()
            .ok_or_else(|| anyhow!("CLI app-server request must have an id"))?;
        self.write_request(&request).await?;

        loop {
            match self.read_output().await? {
                StdioOutput::Event { event } => self.handle_event(*event).await?,
                StdioOutput::Response {
                    id,
                    ok,
                    output,
                    error,
                } => {
                    if id.as_deref() == Some(target_id.as_str()) {
                        return decode_response(ok, output, error);
                    }
                    let Some(response_id) = id else {
                        bail!("app-server returned an uncorrelated response")
                    };
                    if !self.pending_control_responses.remove(&response_id) {
                        bail!("app-server returned unexpected response id: {response_id}")
                    }
                    decode_response(ok, output, error)?;
                }
                _ => bail!("app-server returned an unsupported output variant"),
            }
        }
    }

    async fn handle_event(&mut self, event: AppServerEvent) -> Result<()> {
        match event {
            AppServerEvent::ApprovalRequested { request } => {
                let (approved, note) = prompt_approval(&request, self.interactive_terminal)?;
                let id = self.request_id("approval");
                self.write_request(&StdioRequest::Approval {
                    id: Some(id.clone()),
                    approval_id: request.approval_id,
                    approved,
                    note,
                    cache: ApprovalCacheScope::None,
                })
                .await?;
                self.pending_control_responses.insert(id);
            }
            AppServerEvent::UserInputRequested { request } => {
                let response = prompt_user_input(&request, self.interactive_terminal)?;
                let id = self.request_id("user-input");
                self.write_request(&StdioRequest::UserInput {
                    id: Some(id.clone()),
                    request_id: request.request_id,
                    response,
                })
                .await?;
                self.pending_control_responses.insert(id);
            }
            AppServerEvent::EventStreamLagged { count } => {
                eprintln!("warning: app-server event stream lost {count} events");
            }
            AppServerEvent::Runtime { .. }
            | AppServerEvent::UserMessageSubmitted { .. }
            | AppServerEvent::TurnOutput { .. }
            | AppServerEvent::ApprovalResolved { .. }
            | AppServerEvent::UserInputResolved { .. }
            | AppServerEvent::ModulesReloaded { .. }
            | AppServerEvent::SessionActivityUpdated { .. }
            | AppServerEvent::Error { .. }
            | AppServerEvent::Shutdown => {}
            _ => {}
        }
        Ok(())
    }

    async fn write_request(&mut self, request: &StdioRequest) -> Result<()> {
        let mut line = serde_json::to_vec(request).context("encode app-server request")?;
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .await
            .context("write app-server request")?;
        self.stdin.flush().await.context("flush app-server request")
    }

    async fn read_output(&mut self) -> Result<StdioOutput> {
        let line = self
            .stdout
            .next_line()
            .await
            .context("read app-server output")?
            .ok_or_else(|| match self.child.try_wait() {
                Ok(Some(status)) => anyhow!("local app-server exited with {status}"),
                Ok(None) => anyhow!("local app-server closed stdout"),
                Err(error) => anyhow!("local app-server closed stdout: {error}"),
            })?;
        serde_json::from_str(&line).context("decode app-server output")
    }

    fn request_id(&mut self, operation: &str) -> String {
        let id = format!("cli-{operation}-{}", self.next_id);
        self.next_id += 1;
        id
    }
}

fn decode_response(
    ok: bool,
    output: Option<Value>,
    error: Option<String>,
) -> Result<Option<Value>> {
    if ok {
        return Ok(output);
    }
    bail!(
        "{}",
        error.unwrap_or_else(|| "app-server request failed without an error message".to_owned())
    )
}

fn permission_mode_arg(mode: PermissionMode) -> Result<&'static str> {
    match mode {
        PermissionMode::Plan => Ok("plan"),
        PermissionMode::Normal => Ok("normal"),
        PermissionMode::Auto => Ok("auto"),
        _ => bail!("unsupported permission mode for product CLI"),
    }
}

fn prompt_approval(
    request: &proteus_contracts::app_protocol::AppApprovalRequest,
    interactive: bool,
) -> Result<(bool, Option<String>)> {
    if !interactive {
        return Ok((
            false,
            Some(format!(
                "approval transport is not interactive: {}",
                request.reason
            )),
        ));
    }

    clear_terminal_line()?;
    let args = request.call.args.to_string();
    let args = if args.chars().count() > 500 {
        format!("{}...", args.chars().take(500).collect::<String>())
    } else {
        args
    };
    eprintln!();
    eprintln!("Approval requested");
    if let Some(label) = request
        .origin
        .as_ref()
        .and_then(|origin| origin.label.as_deref())
    {
        eprintln!("from: subagent '{label}'");
    }
    eprintln!("tool: {}", request.call.name);
    eprintln!("cwd: {}", request.cwd.display());
    eprintln!("reason: {}", request.reason);
    if let Some(spec) = &request.tool_spec {
        eprintln!("safety: {:?}", spec.safety);
    }
    eprintln!("args: {args}");
    eprint!("Approve this tool call? [y/N] ");
    io::stderr().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let approved = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    let note = (!approved).then(|| format!("tool call was not approved: {}", request.reason));
    Ok((approved, note))
}

fn prompt_user_input(request: &UserInputRequest, interactive: bool) -> Result<UserInputResponse> {
    if !interactive {
        return Ok(UserInputResponse::empty());
    }

    clear_terminal_line()?;
    eprintln!();
    eprintln!("{}", request.title.as_deref().unwrap_or("Input requested"));
    let mut answers = HashMap::new();
    for question in &request.questions {
        eprintln!("{}: {}", question.header, question.question);
        for (index, option) in question.options.iter().enumerate() {
            eprintln!("  {}. {} — {}", index + 1, option.label, option.description);
        }
        if question.is_secret {
            eprintln!("  warning: terminal input is not hidden");
        }
        let hint = if question.multi_select {
            "answer (comma-separated): "
        } else {
            "answer: "
        };
        eprint!("{hint}");
        io::stderr().flush()?;

        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let raw_answers = if question.multi_select {
            line.trim().split(',').map(str::trim).collect::<Vec<_>>()
        } else {
            vec![line.trim()]
        };
        let values = raw_answers
            .into_iter()
            .filter(|answer| !answer.is_empty())
            .map(|answer| {
                answer
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| question.options.get(index))
                    .map(|option| option.label.clone())
                    .unwrap_or_else(|| answer.to_owned())
            })
            .collect();
        answers.insert(question.id.clone(), UserInputAnswer::new(values));
    }
    Ok(UserInputResponse::new(answers))
}

fn clear_terminal_line() -> Result<()> {
    print!("\r\x1b[2K");
    io::stdout().flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_modes_have_exact_cli_values() {
        assert_eq!(permission_mode_arg(PermissionMode::Plan).unwrap(), "plan");
        assert_eq!(
            permission_mode_arg(PermissionMode::Normal).unwrap(),
            "normal"
        );
        assert_eq!(permission_mode_arg(PermissionMode::Auto).unwrap(), "auto");
    }

    #[test]
    fn successful_response_preserves_output() {
        let value = serde_json::json!({"answer": 42});
        assert_eq!(
            decode_response(true, Some(value.clone()), None).unwrap(),
            Some(value)
        );
        assert!(decode_response(false, None, Some("failed".to_owned())).is_err());
    }

    #[test]
    fn user_input_maps_numeric_options_and_free_text() {
        let request =
            UserInputRequest::new("input-1", std::path::PathBuf::from("/workspace"), vec![]);
        assert!(
            prompt_user_input(&request, false)
                .unwrap()
                .answers
                .is_empty()
        );
    }
}
