use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use proteus_process_host::{
    ContentLengthFraming, ProcessHost, ProcessSession, ProcessSpec, ReceiveFrameError,
};
use serde_json::{Value, json};
use url::Url;

use crate::{
    diagnostics::{DiagnosticReport, parse_publish_diagnostics},
    path::RustDocument,
};

const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug)]
pub(crate) struct RustAnalyzerConfig {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) initialize_timeout: Duration,
    pub(crate) diagnostics_timeout: Duration,
}

pub(crate) struct RustAnalyzerWorkspace {
    root: PathBuf,
    root_uri: String,
    command: String,
    diagnostics_timeout: Duration,
    host: ProcessHost<ContentLengthFraming>,
    documents: HashMap<String, i64>,
}

impl RustAnalyzerWorkspace {
    pub(crate) fn new(root: PathBuf, config: RustAnalyzerConfig) -> Result<Self> {
        let root_uri = Url::from_directory_path(&root)
            .map_err(|_| anyhow!("failed to convert {} to a workspace URI", root.display()))?
            .to_string();
        let root_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace")
            .to_owned();
        let initialize_root_uri = root_uri.clone();
        let initialize_root_name = root_name.clone();
        let initialize_timeout = config.initialize_timeout;
        let spec = ProcessSpec::new(config.command.clone())
            .args(config.args.clone())
            .env_allowlist(["HOME", "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN"])
            .cwd(&root);
        let host =
            ProcessHost::with_initializer(spec, ContentLengthFraming::default(), move |session| {
                session.request(
                    "initialize",
                    json!({
                        "processId": Value::Null,
                        "clientInfo": {
                            "name": "proteus-rust-lsp",
                            "version": env!("CARGO_PKG_VERSION")
                        },
                        "rootUri": initialize_root_uri,
                        "capabilities": {
                            "workspace": {
                                "configuration": true,
                                "workspaceFolders": true
                            },
                            "textDocument": {
                                "publishDiagnostics": {
                                    "relatedInformation": true,
                                    "versionSupport": true,
                                    "codeDescriptionSupport": true,
                                    "dataSupport": true
                                }
                            }
                        },
                        "workspaceFolders": [{
                            "uri": initialize_root_uri,
                            "name": initialize_root_name
                        }],
                        "trace": "off"
                    }),
                    initialize_timeout,
                )?;
                session.notify("initialized", json!({}))
            });

        Ok(Self {
            root,
            root_uri,
            command: config.command,
            diagnostics_timeout: config.diagnostics_timeout,
            host,
            documents: HashMap::new(),
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn diagnostics(
        &mut self,
        document: &RustDocument,
        mut is_cancelled: impl FnMut() -> Result<bool>,
    ) -> Result<DiagnosticReport> {
        if is_cancelled()? {
            bail!("lsp_diagnostics invocation was canceled");
        }
        let previous_version = self.documents.get(&document.uri).copied();
        let document_version = previous_version.unwrap_or(0) + 1;
        let result = (|| {
            let mut session = self.host.ensure_session().with_context(|| {
                format!(
                    "rust-analyzer is unavailable or failed to initialize via '{}'",
                    self.command
                )
            })?;
            session.drain_notifications();
            if previous_version.is_some() {
                session.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": {
                            "uri": document.uri,
                            "version": document_version
                        },
                        "contentChanges": [{ "text": document.text }]
                    }),
                )?;
            } else {
                session.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": document.uri,
                            "languageId": "rust",
                            "version": document_version,
                            "text": document.text
                        }
                    }),
                )?;
            }
            wait_for_diagnostics(
                &mut session,
                &self.root_uri,
                &document.uri,
                document_version,
                self.diagnostics_timeout,
                &mut is_cancelled,
            )
        })();

        match result {
            Ok(report) => {
                self.documents
                    .insert(document.uri.clone(), document_version);
                Ok(report)
            }
            Err(error) => {
                self.host.reset();
                self.documents.clear();
                Err(error)
            }
        }
    }
}

fn wait_for_diagnostics(
    session: &mut ProcessSession<ContentLengthFraming>,
    root_uri: &str,
    document_uri: &str,
    document_version: i64,
    timeout: Duration,
    is_cancelled: &mut impl FnMut() -> Result<bool>,
) -> Result<DiagnosticReport> {
    let started = Instant::now();
    let mut latest = None;
    loop {
        if is_cancelled()? {
            bail!("lsp_diagnostics invocation was canceled");
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            if let Some(report) = latest {
                return Ok(report);
            }
            bail!(
                "rust-analyzer did not publish diagnostics within {}ms",
                timeout.as_millis()
            );
        }
        let wait = (timeout - elapsed).min(POLL_INTERVAL);
        let message = match session.recv_frame(wait) {
            Ok(message) => message,
            Err(ReceiveFrameError::Timeout { .. }) if latest.is_some() => {
                return latest.ok_or_else(|| anyhow!("diagnostic report disappeared"));
            }
            Err(ReceiveFrameError::Timeout { .. }) => continue,
            Err(error) => return Err(error.into()),
        };
        if handle_server_request(session, &message, root_uri)? {
            continue;
        }
        if let Some(report) = parse_publish_diagnostics(&message, document_uri, document_version)? {
            latest = Some(report);
        }
    }
}

fn handle_server_request(
    session: &mut ProcessSession<ContentLengthFraming>,
    message: &Value,
    root_uri: &str,
) -> Result<bool> {
    let Some(id) = message.get("id").cloned() else {
        return Ok(false);
    };
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Ok(false);
    };
    let result = match method {
        "workspace/configuration" => {
            let count = message
                .pointer("/params/items")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array((0..count).map(|_| Value::Null).collect())
        }
        "workspace/workspaceFolders" => json!([{
            "uri": root_uri,
            "name": workspace_name(root_uri)
        }]),
        "workspace/applyEdit" => json!({
            "applied": false,
            "failureReason": "lsp_diagnostics is read-only"
        }),
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create"
        | "window/showMessageRequest" => Value::Null,
        _ => {
            session.send_frame(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Proteus LSP client does not implement {method}")
                }
            }))?;
            return Ok(true);
        }
    };
    session.send_frame(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))?;
    Ok(true)
}

fn workspace_name(root_uri: &str) -> String {
    Url::parse(root_uri)
        .ok()
        .and_then(|uri| {
            uri.path_segments().and_then(|segments| {
                segments
                    .rev()
                    .find(|segment| !segment.is_empty())
                    .map(str::to_owned)
            })
        })
        .unwrap_or_else(|| "workspace".to_owned())
}
