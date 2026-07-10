use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpServerConfig {
    pub bind: SocketAddr,
    pub session_token: String,
    pub require_session_token: bool,
    pub allowed_origins: Vec<String>,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787),
            session_token: new_session_token(),
            require_session_token: false,
            allowed_origins: default_allowed_origins(),
        }
    }
}

impl HttpServerConfig {
    pub fn validate(&self) -> Result<()> {
        if self.require_session_token && self.session_token.is_empty() {
            bail!("HTTP session token must not be empty when token auth is enabled");
        }
        if !self.bind.ip().is_loopback() && !self.require_session_token {
            bail!("non-loopback HTTP bind {} requires --token", self.bind.ip());
        }
        Ok(())
    }
}

pub(super) fn new_session_token() -> String {
    new_http_token()
}

pub(super) fn new_request_id() -> String {
    new_http_token()
}

fn new_http_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub(super) fn default_allowed_origins() -> Vec<String> {
    vec![
        "http://127.0.0.1:1420".to_owned(),
        "http://localhost:1420".to_owned(),
        "http://127.0.0.1:1421".to_owned(),
        "http://localhost:1421".to_owned(),
    ]
}
