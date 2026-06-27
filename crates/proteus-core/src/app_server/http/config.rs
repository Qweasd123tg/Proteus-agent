use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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
