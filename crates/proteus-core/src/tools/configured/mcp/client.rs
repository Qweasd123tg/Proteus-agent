//! Persistent tools-only MCP client. rmcp owns negotiation and RPC routing;
//! Proteus owns admission, deadlines and the lifetime of the child generation.

use std::{
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use proteus_process_host::ProcessSpec;
use rmcp::{
    RoleClient, ServiceExt,
    model::{
        CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
        ClientCapabilities, ClientInfo, ClientRequest, Implementation, ProtocolVersion,
        ServerResult,
    },
    service::{PeerRequestOptions, RunningService, ServiceError},
};
use serde_json::Value;
use tokio::{runtime::Runtime, sync::Mutex, time::Instant};

use crate::contracts::CancellationToken;

use super::transport::{ConnectionControl, McpTransport};

pub(in crate::tools::configured) struct McpStdioHost {
    timeout: Duration,
    state: Arc<ClientState>,
}

struct ClientState {
    spec: ProcessSpec,
    info: ClientInfo,
    max_response_bytes: usize,
    connection: Mutex<Option<Arc<Connection>>>,
}

struct Connection {
    service: RunningService<RoleClient, ClientInfo>,
    control: ConnectionControl,
}

impl Connection {
    fn is_closed(&self) -> bool {
        self.service.is_closed() || self.control.is_closed()
    }

    fn explain(&self, error: impl std::fmt::Display) -> anyhow::Error {
        match self.control.error() {
            Some(detail) => anyhow!("MCP {error}: {detail}"),
            None => anyhow!("MCP {error}"),
        }
    }
}

impl McpStdioHost {
    pub(super) fn new(
        spec: ProcessSpec,
        protocol_version: String,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self> {
        let version: ProtocolVersion =
            serde_json::from_value(Value::String(protocol_version.clone()))?;
        if !ProtocolVersion::KNOWN_VERSIONS.contains(&version) {
            bail!("unsupported MCP protocol_version: {protocol_version}");
        }
        Ok(Self {
            timeout,
            state: Arc::new(ClientState {
                spec,
                info: ClientInfo::new(
                    ClientCapabilities::default(),
                    Implementation::new("proteus-core", env!("CARGO_PKG_VERSION")),
                )
                .with_protocol_version(version),
                max_response_bytes,
                connection: Mutex::new(None),
            }),
        })
    }

    pub(super) fn timeout(&self) -> Duration {
        self.timeout
    }

    pub(super) fn list_tools(&self) -> Result<Vec<rmcp::model::Tool>> {
        let state = self.state.clone();
        let timeout = self.timeout;
        run_sync(async move {
            let deadline = Instant::now() + timeout;
            let connection = tokio::time::timeout_at(deadline, state.connect())
                .await
                .map_err(|_| timeout_error(timeout))??;
            match tokio::time::timeout_at(deadline, connection.service.list_all_tools()).await {
                Ok(Ok(tools)) => Ok(tools),
                Ok(Err(error)) => {
                    state.handle_error(&connection, &error).await?;
                    Err(connection.explain(error))
                }
                Err(_) => {
                    state.invalidate(&connection).await?;
                    Err(timeout_error(timeout))
                }
            }
        })
    }

    pub(super) async fn call_tool(
        &self,
        remote_tool: String,
        args: Value,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult> {
        let Value::Object(arguments) = args else {
            bail!("MCP tool arguments must be a JSON object");
        };
        let cancellation = cancellation.child_token();
        let _cancel_on_drop = CancelOnDrop(cancellation.clone());
        let state = self.state.clone();
        runtime()?.spawn(async move {
            let deadline = Instant::now() + timeout;
            let connection = tokio::select! {
                biased;
                _ = cancellation.cancelled() => bail!("MCP tool call canceled"),
                _ = tokio::time::sleep_until(deadline) => return Err(timeout_error(timeout)),
                result = state.connect() => result?,
            };
            let mut request_id = None;
            let result = {
                let request = async {
                    let params = CallToolRequestParams::new(remote_tool).with_arguments(arguments);
                    // One admitted tool call produces one request. Do not run
                    // SDK MRTR/task continuations outside ToolRegistry/policy.
                    let handle = connection.service.send_cancellable_request(
                        ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                        PeerRequestOptions::no_options(),
                    ).await?;
                    request_id = Some(handle.id.clone());
                    handle.await_response().await
                };
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => Err(ServiceError::Cancelled { reason: Some("tool call canceled".into()) }),
                    _ = tokio::time::sleep_until(deadline) => Err(ServiceError::Timeout { timeout }),
                    result = request => result,
                }
            };
            match result {
                Ok(ServerResult::CallToolResult(result)) => Ok(result),
                Ok(_) => {
                    state.invalidate(&connection).await?;
                    bail!("unsupported MCP tools/call response: expected a completed tool result");
                }
                Err(error) => {
                    if matches!(error, ServiceError::Cancelled { .. } | ServiceError::Timeout { .. }) {
                        if let Some(id) = request_id {
                            let _ = tokio::time::timeout(Duration::from_millis(100), connection.service.notify_cancelled(
                                CancelledNotificationParam::new(Some(id), Some(error.to_string()))
                            )).await;
                        }
                        state.invalidate(&connection).await?;
                        if matches!(error, ServiceError::Timeout { .. }) {
                            return Err(timeout_error(timeout));
                        }
                    } else {
                        state.handle_error(&connection, &error).await?;
                    }
                    Err(connection.explain(error))
                }
            }
        }).await.context("MCP client task failed")?
    }
}

impl ClientState {
    async fn connect(&self) -> Result<Arc<Connection>> {
        let mut current = self.connection.lock().await;
        if let Some(connection) = current
            .as_ref()
            .filter(|connection| !connection.is_closed())
        {
            return Ok(connection.clone());
        }
        if let Some(previous) = current.take() {
            previous.control.terminate().await?;
        }
        let transport = McpTransport::spawn(&self.spec, self.max_response_bytes)?;
        let control = transport.control();
        let service =
            self.info
                .clone()
                .serve(transport)
                .await
                .map_err(|error| match control.error() {
                    Some(detail) => anyhow!("MCP initialize failed: {error}: {detail}"),
                    None => anyhow!("MCP initialize failed: {error}"),
                })?;
        let peer_info = service
            .peer_info()
            .context("MCP server did not return initialize information")?;
        if !ProtocolVersion::KNOWN_VERSIONS.contains(&peer_info.protocol_version) {
            control.terminate().await?;
            bail!(
                "unsupported MCP server protocol version: {}",
                peer_info.protocol_version
            );
        }
        if peer_info.capabilities.tools.is_none() {
            control.terminate().await?;
            bail!("MCP server did not advertise tools capability");
        }
        let connection = Arc::new(Connection { service, control });
        *current = Some(connection.clone());
        Ok(connection)
    }

    async fn handle_error(&self, connection: &Arc<Connection>, error: &ServiceError) -> Result<()> {
        // Application/RPC errors do not imply the process is broken.
        if !matches!(error, ServiceError::McpError(_)) || connection.is_closed() {
            self.invalidate(connection).await?;
        }
        Ok(())
    }

    async fn invalidate(&self, connection: &Arc<Connection>) -> Result<()> {
        let mut current = self.connection.lock().await;
        if current
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, connection))
        {
            *current = None;
        }
        // Finish termination before allowing another caller to (re)spawn.
        connection.control.terminate().await
    }
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn timeout_error(timeout: Duration) -> anyhow::Error {
    anyhow!(
        "MCP server did not respond within {}ms",
        timeout.as_millis()
    )
}

fn runtime() -> Result<&'static Runtime> {
    // Discovery is a synchronous assembly boundary, including callers already
    // inside Tokio. An independent runtime keeps persistent SDK IO alive across
    // that boundary without nested block_on or per-invocation runtime creation.
    static RUNTIME: OnceLock<std::io::Result<Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("proteus-mcp")
                .enable_all()
                .build()
        })
        .as_ref()
        .map_err(|error| anyhow!("failed to start MCP runtime: {error}"))
}

fn run_sync<T: Send + 'static>(
    future: impl Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    runtime()?.spawn(async move {
        let _ = sender.send(future.await);
    });
    receiver.recv().context("MCP discovery task failed")?
}
