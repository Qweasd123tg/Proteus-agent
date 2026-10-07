use super::paths::{contained, expand};
use crate::domain::{ConfiguredMcpServerConfig, ProcessEnvironmentConfig, ToolSafety};
use anyhow::{Result, bail};
use hyper::http;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub(super) const MCP_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct McpFile {
    #[serde(rename = "$schema")]
    pub schema: String,
    #[serde(rename = "mcpServers")]
    pub servers: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum Server {
    #[serde(rename = "stdio")]
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        cwd: Option<String>,
    },
    #[serde(rename = "streamable-http")]
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    #[serde(rename = "sse")]
    Sse {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

pub(super) fn server(
    name: &str,
    value: Value,
    root: &Path,
    data: &Path,
    enabled: bool,
) -> Result<Option<ConfiguredMcpServerConfig>> {
    if value.get("cwd").is_some_and(Value::is_null) {
        bail!("MCP cwd must be a string when present");
    }
    match serde_json::from_value(value)? {
        Server::Stdio {
            command,
            args,
            env,
            cwd,
        } => {
            if command.is_empty() {
                bail!("MCP command must not be empty");
            }
            let command = if command.starts_with("./") {
                let executable = contained(root, &root.join(&command))?;
                if !executable.is_file() {
                    bail!("MCP command is not a file");
                }
                executable.to_string_lossy().into_owned()
            } else {
                if command.contains(['/', '\\']) {
                    bail!("MCP command must be bare or start with ./");
                }
                command
            };
            if env.keys().any(|key| {
                key.eq_ignore_ascii_case("PLUGIN_ROOT") || key.eq_ignore_ascii_case("PLUGIN_DATA")
            }) {
                bail!("reserved plugin environment variable");
            }
            let cwd = match cwd {
                None => root.to_path_buf(),
                Some(cwd) => {
                    let base = if cwd.starts_with("./")
                        || cwd == "${PLUGIN_ROOT}"
                        || cwd.starts_with("${PLUGIN_ROOT}/")
                    {
                        root
                    } else if cwd == "${PLUGIN_DATA}" || cwd.starts_with("${PLUGIN_DATA}/") {
                        data
                    } else {
                        bail!(
                            "MCP cwd must be plugin-relative or rooted in PLUGIN_ROOT/PLUGIN_DATA"
                        );
                    };
                    let expanded = expand(&cwd, root, data);
                    let path = if cwd.starts_with("./") {
                        root.join(expanded)
                    } else {
                        expanded.into()
                    };
                    contained(base, &path)?
                }
            };
            let mut env = env
                .into_iter()
                .map(|(key, value)| (key, expand(&value, root, data)))
                .collect::<BTreeMap<_, _>>();
            env.insert("PLUGIN_ROOT".into(), root.to_string_lossy().into_owned());
            env.insert("PLUGIN_DATA".into(), data.to_string_lossy().into_owned());
            Ok(Some(ConfiguredMcpServerConfig {
                name: name.into(),
                command,
                enabled,
                cwd: Some(cwd),
                args: args.iter().map(|value| expand(value, root, data)).collect(),
                environment: ProcessEnvironmentConfig {
                    env_allowlist: vec!["PATH".into()],
                    env,
                },
                protocol_version: crate::domain::mcp_protocol_version(),
                safety: ToolSafety::RunsCommands,
                supports_parallel_tool_calls: false,
                timeout_ms: None,
                max_response_bytes: None,
                metadata: Value::Null,
            }))
        }
        Server::Http { url, headers } | Server::Sse { url, headers } => {
            let uri: http::Uri = url.parse()?;
            if url.contains('#') || uri.authority().is_some_and(|a| a.as_str().contains('@')) {
                bail!("invalid remote MCP URL");
            }
            let host = uri
                .host()
                .ok_or_else(|| anyhow::anyhow!("MCP URL must be absolute"))?;
            let loopback = host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback());
            if uri.scheme_str() != Some("https") && !(uri.scheme_str() == Some("http") && loopback)
            {
                bail!("remote MCP requires HTTPS except on loopback");
            }
            let mut names = std::collections::BTreeSet::new();
            for (name, value) in headers {
                let header: http::header::HeaderName = name.parse()?;
                let _: http::header::HeaderValue = value.parse()?;
                if !names.insert(header.as_str().to_owned()) {
                    bail!("duplicate MCP header");
                }
            }
            Ok(None)
        }
    }
}
