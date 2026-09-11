//! A2A transport for complete, configured Proteus sessions.
//! The SDK owns wire types, JSON-RPC dispatch and SSE encoding. This adapter
//! owns the mapping to session admission, policy requests and run settlement.

use std::{path::PathBuf, sync::Arc};

use a2a_server::agent_card::{StaticAgentCard, agent_card_router};
use anyhow::Result;
use tokio::net::TcpListener;

use crate::core::AppConfig;

mod card;
mod handler;
mod interaction;
mod runtime;
mod state;
#[cfg(test)]
mod tests;
mod validation;

/// Local peer endpoint. Task/context identities live until this process exits.
#[derive(Debug, Clone)]
pub struct A2aServerConfig {
    pub port: u16,
    pub ready_stdout: bool,
    pub max_contexts: usize,
    pub max_tasks: usize,
}

impl Default for A2aServerConfig {
    fn default() -> Self {
        Self {
            port: 0,
            ready_stdout: false,
            max_contexts: 32,
            max_tasks: 1024,
        }
    }
}

pub async fn run_a2a_app_server(
    config: AppConfig,
    cwd: PathBuf,
    config_path: Option<PathBuf>,
    options: A2aServerConfig,
) -> Result<()> {
    anyhow::ensure!(
        options.max_contexts > 0 && options.max_tasks > 0,
        "A2A limits must be positive"
    );
    anyhow::ensure!(
        config_path.is_some(),
        "A2A server requires a config path for session storage"
    );
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, options.port)).await?;
    let url = format!("http://{}/", listener.local_addr()?);
    let service = Arc::new(state::Service::new(
        config,
        cwd,
        config_path,
        options.clone(),
    ));
    let router = a2a_server::jsonrpc::jsonrpc_router(service.clone()).merge(agent_card_router(
        Arc::new(StaticAgentCard::new(card::agent_card(url.clone()))),
    ));
    if options.ready_stdout {
        use std::io::Write;
        println!("{}", serde_json::json!({"type": "a2a_ready", "url": url}));
        std::io::stdout().flush()?;
    }
    eprintln!("Proteus A2A listening on {url}");
    let shutdown_service = service.clone();
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown_service.shutdown().await;
        })
        .await;
    service.shutdown().await;
    result?;
    Ok(())
}
