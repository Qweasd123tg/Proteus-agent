use anyhow::{Result, bail};
use proteus_core::app_server::http::HttpServerConfig;

pub(crate) fn is_modules_list_command(task: &[String]) -> bool {
    matches!(task, [module, command] if module == "modules" && command == "list")
}

pub(crate) fn parse_eval_report_command(task: &[String]) -> Result<Option<&str>> {
    match task {
        [namespace, command, path] if namespace == "eval" && command == "report" => Ok(Some(path)),
        [namespace, command, ..] if namespace == "eval" && command == "report" => {
            bail!("usage: proteus eval report <event-log-path>")
        }
        [namespace, ..] if namespace == "eval" => {
            bail!("usage: proteus eval report <event-log-path>")
        }
        _ => Ok(None),
    }
}

pub(crate) fn is_tools_list_command(task: &[String]) -> bool {
    matches!(task, [tool, command] if tool == "tools" && command == "list")
}

pub(crate) fn is_app_server_stdio_command(task: &[String]) -> bool {
    matches!(task, [server, transport] if server == "server" && transport == "stdio")
}

pub(crate) fn parse_app_server_http_command(task: &[String]) -> Result<Option<HttpServerConfig>> {
    let [server, transport, rest @ ..] = task else {
        return Ok(None);
    };
    if server != "server" || transport != "http" {
        return Ok(None);
    }

    let mut config = HttpServerConfig::default();
    let mut host = config.bind.ip();
    let mut port = config.bind.port();
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", app_server_http_usage()))?;
                host = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid --host value: {value}"))?;
            }
            "--port" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", app_server_http_usage()))?;
                port = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid --port value: {value}"))?;
            }
            "--token" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", app_server_http_usage()))?;
                if value.is_empty() {
                    bail!("--token must not be empty");
                }
                config.session_token = value.clone();
                config.require_session_token = true;
            }
            "--allow-origin" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", app_server_http_usage()))?;
                config.allowed_origins.push(value.clone());
            }
            _ => bail!("{}", app_server_http_usage()),
        }
    }
    config.bind = std::net::SocketAddr::new(host, port);
    Ok(Some(config))
}

fn app_server_http_usage() -> &'static str {
    "usage: proteus server http [--host <ip>] [--port <port>] [--token <token>] [--allow-origin <origin>]"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectTopologyFormat {
    Table,
    Json,
    Markdown,
    Runtime,
    RuntimeMermaid,
    Map,
    Mermaid,
}

pub(crate) fn parse_inspect_topology_command(
    task: &[String],
) -> Result<Option<InspectTopologyFormat>> {
    let [namespace, rest @ ..] = task else {
        return Ok(None);
    };
    if namespace != "inspect" {
        return Ok(None);
    }

    match rest {
        [] => Ok(Some(InspectTopologyFormat::Markdown)),
        [command, args @ ..] if command == "topology" => {
            Ok(Some(parse_inspect_topology_format(args)?))
        }
        _ => bail!("{}", inspect_topology_usage()),
    }
}

fn parse_inspect_topology_format(args: &[String]) -> Result<InspectTopologyFormat> {
    let mut format = InspectTopologyFormat::Markdown;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--format" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", inspect_topology_usage()))?;
                format = inspect_topology_format_value(value)?;
            }
            value if value.starts_with("--format=") => {
                let value = value
                    .strip_prefix("--format=")
                    .expect("starts_with checked");
                format = inspect_topology_format_value(value)?;
            }
            _ => bail!("{}", inspect_topology_usage()),
        }
    }
    Ok(format)
}

fn inspect_topology_format_value(value: &str) -> Result<InspectTopologyFormat> {
    match value {
        "table" => Ok(InspectTopologyFormat::Table),
        "json" => Ok(InspectTopologyFormat::Json),
        "markdown" | "md" => Ok(InspectTopologyFormat::Markdown),
        "runtime" | "path" => Ok(InspectTopologyFormat::Runtime),
        "runtime-mermaid" | "runtime_mmd" | "runtime-mmd" => {
            Ok(InspectTopologyFormat::RuntimeMermaid)
        }
        "map" => Ok(InspectTopologyFormat::Map),
        "mermaid" | "mmd" => Ok(InspectTopologyFormat::Mermaid),
        _ => bail!(
            "unknown topology format '{value}', expected table, json, markdown, runtime, runtime-mermaid, map, or mermaid"
        ),
    }
}

fn inspect_topology_usage() -> &'static str {
    "usage: proteus inspect [topology] [--format table|json|markdown|runtime|runtime-mermaid|map|mermaid]"
}

pub(crate) fn is_doctor_command(task: &[String]) -> bool {
    matches!(task, [command] if command == "doctor")
}
