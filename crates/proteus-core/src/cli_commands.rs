use std::path::PathBuf;

use anyhow::{Result, bail};
use proteus_core::app_server::http::HttpServerConfig;
use proteus_core::domain::{ExchangeId, TurnId};

pub(crate) fn is_modules_list_command(task: &[String]) -> bool {
    matches!(task, [module, command] if module == "modules" && command == "list")
}

pub(crate) fn parse_eval_report_command(task: &[String]) -> Result<Option<&str>> {
    match task {
        [namespace, command, path] if namespace == "eval" && command == "report" => Ok(Some(path)),
        [namespace, command, ..] if namespace == "eval" && command == "report" => {
            bail!("usage: proteus eval report <session-dir-or-journal-path>")
        }
        [namespace, ..] if namespace == "eval" => {
            bail!("usage: proteus eval report <session-dir-or-journal-path>")
        }
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptReplayCommand {
    pub source: PathBuf,
    pub exchange_id: Option<ExchangeId>,
    pub allow_hosted_tools: bool,
    pub json: bool,
}

pub(crate) fn parse_prompt_replay_command(task: &[String]) -> Result<Option<PromptReplayCommand>> {
    let Some(namespace) = task.first() else {
        return Ok(None);
    };
    if namespace != "replay" {
        return Ok(None);
    }
    if task.get(1).map(String::as_str) != Some("prompt") {
        return Ok(None);
    }

    let mut source = None;
    let mut exchange_id = None;
    let mut allow_hosted_tools = false;
    let mut json = false;
    let mut index = 2;
    while index < task.len() {
        let argument = &task[index];
        match argument.as_str() {
            "--exchange-id" => {
                if exchange_id.is_some() {
                    bail!("--exchange-id may be specified only once");
                }
                index += 1;
                let value = task
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("{}", prompt_replay_usage()))?;
                exchange_id = Some(parse_exchange_id(value)?);
            }
            value if value.starts_with("--exchange-id=") => {
                if exchange_id.is_some() {
                    bail!("--exchange-id may be specified only once");
                }
                let value = value
                    .strip_prefix("--exchange-id=")
                    .expect("starts_with checked");
                exchange_id = Some(parse_exchange_id(value)?);
            }
            "--allow-hosted-tools" => {
                if allow_hosted_tools {
                    bail!("--allow-hosted-tools may be specified only once");
                }
                allow_hosted_tools = true;
            }
            "--json" => {
                if json {
                    bail!("--json may be specified only once");
                }
                json = true;
            }
            value if value.starts_with('-') => bail!("{}", prompt_replay_usage()),
            value if source.is_none() => source = Some(PathBuf::from(value)),
            _ => bail!("{}", prompt_replay_usage()),
        }
        index += 1;
    }

    let source = source.ok_or_else(|| anyhow::anyhow!("{}", prompt_replay_usage()))?;
    Ok(Some(PromptReplayCommand {
        source,
        exchange_id,
        allow_hosted_tools,
        json,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkflowReplayCommand {
    pub source: PathBuf,
    pub turn_id: Option<TurnId>,
    pub json: bool,
}

pub(crate) fn parse_workflow_replay_command(
    task: &[String],
) -> Result<Option<WorkflowReplayCommand>> {
    let Some(namespace) = task.first() else {
        return Ok(None);
    };
    if namespace != "replay" {
        return Ok(None);
    }
    match task.get(1).map(String::as_str) {
        Some("prompt") => return Ok(None),
        Some("workflow") => {}
        _ => bail!("{}", replay_usage()),
    }

    let mut source = None;
    let mut turn_id = None;
    let mut json = false;
    let mut index = 2;
    while index < task.len() {
        let argument = &task[index];
        match argument.as_str() {
            "--turn-id" => {
                if turn_id.is_some() {
                    bail!("--turn-id may be specified only once");
                }
                index += 1;
                let value = task
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("{}", workflow_replay_usage()))?;
                turn_id = Some(parse_turn_id(value)?);
            }
            value if value.starts_with("--turn-id=") => {
                if turn_id.is_some() {
                    bail!("--turn-id may be specified only once");
                }
                let value = value
                    .strip_prefix("--turn-id=")
                    .expect("starts_with checked");
                turn_id = Some(parse_turn_id(value)?);
            }
            "--json" => {
                if json {
                    bail!("--json may be specified only once");
                }
                json = true;
            }
            value if value.starts_with('-') => bail!("{}", workflow_replay_usage()),
            value if source.is_none() => source = Some(PathBuf::from(value)),
            _ => bail!("{}", workflow_replay_usage()),
        }
        index += 1;
    }

    let source = source.ok_or_else(|| anyhow::anyhow!("{}", workflow_replay_usage()))?;
    Ok(Some(WorkflowReplayCommand {
        source,
        turn_id,
        json,
    }))
}

fn parse_exchange_id(value: &str) -> Result<ExchangeId> {
    value
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid --exchange-id '{value}': {error}"))
}

fn parse_turn_id(value: &str) -> Result<TurnId> {
    value
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid --turn-id '{value}': {error}"))
}

fn prompt_replay_usage() -> &'static str {
    "usage: proteus replay prompt <session-dir-or-journal-path> [--exchange-id <id>] [--allow-hosted-tools] [--json]"
}

fn workflow_replay_usage() -> &'static str {
    "usage: proteus replay workflow <session-dir-or-journal-path> [--turn-id <id>] [--json]"
}

fn replay_usage() -> &'static str {
    "usage: proteus replay <prompt|workflow> ..."
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
    config.validate()?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectPlanFormat {
    Text,
    Json,
}

pub(crate) fn parse_inspect_plan_command(task: &[String]) -> Result<Option<InspectPlanFormat>> {
    let [namespace, command, args @ ..] = task else {
        return Ok(None);
    };
    if namespace != "inspect" || command != "plan" {
        return Ok(None);
    }

    let mut format = InspectPlanFormat::Text;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--format" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", inspect_plan_usage()))?;
                format = inspect_plan_format_value(value)?;
            }
            value if value.starts_with("--format=") => {
                let value = value
                    .strip_prefix("--format=")
                    .expect("starts_with checked");
                format = inspect_plan_format_value(value)?;
            }
            _ => bail!("{}", inspect_plan_usage()),
        }
    }
    Ok(Some(format))
}

fn inspect_plan_format_value(value: &str) -> Result<InspectPlanFormat> {
    match value {
        "text" | "table" => Ok(InspectPlanFormat::Text),
        "json" => Ok(InspectPlanFormat::Json),
        _ => bail!("unknown plan format '{value}', expected text or json"),
    }
}

fn inspect_plan_usage() -> &'static str {
    "usage: proteus inspect plan [--format text|json]"
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
        [command, ..] if command == "plan" => Ok(None),
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
