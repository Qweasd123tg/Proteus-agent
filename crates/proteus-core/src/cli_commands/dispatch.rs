use super::*;
use crate::cli_doctor::SessionScope;
use crate::cli_init::{InitProfile, parse_init_command};

pub(crate) enum CliCommand<'a> {
    Init(InitProfile),
    ModulesList,
    ToolsList,
    EvalReport(&'a str),
    PromptReplay(PromptReplayCommand),
    WorkflowReplay(WorkflowReplayCommand),
    InspectPlan(InspectPlanFormat),
    InspectTopology(InspectTopologyFormat),
    Doctor(SessionScope),
    ServerStdio,
    ServerHttp(HttpServerConfig),
    ServerA2a(proteus_core::app_server::a2a::A2aServerConfig),
    Task,
}

/// Classify the entire command before loading config or launching any runtime.
/// Reserved command namespaces must never fall through to model inference.
pub(crate) fn parse_cli_command(task: &[String]) -> Result<CliCommand<'_>> {
    let command = match task.first().map(String::as_str) {
        Some("init") => parse_init_command(task)?.map(CliCommand::Init),
        Some("modules") if is_modules_list_command(task) => Some(CliCommand::ModulesList),
        Some("modules") => bail!("usage: proteus [options] modules list"),
        Some("tools") if is_tools_list_command(task) => Some(CliCommand::ToolsList),
        Some("tools") => bail!("usage: proteus [options] tools list"),
        Some("eval") => parse_eval_report_command(task)?.map(CliCommand::EvalReport),
        Some("replay") => {
            if let Some(command) = parse_prompt_replay_command(task)? {
                Some(CliCommand::PromptReplay(command))
            } else {
                parse_workflow_replay_command(task)?.map(CliCommand::WorkflowReplay)
            }
        }
        Some("inspect") => {
            if let Some(format) = parse_inspect_plan_command(task)? {
                Some(CliCommand::InspectPlan(format))
            } else {
                parse_inspect_topology_command(task)?.map(CliCommand::InspectTopology)
            }
        }
        Some("doctor") => match &task[1..] {
            [] => Some(CliCommand::Doctor(SessionScope::Workspace)),
            [flag] if flag == "--all-sessions" => Some(CliCommand::Doctor(SessionScope::All)),
            _ => bail!("usage: proteus [options] doctor [--all-sessions]"),
        },
        Some("server") if is_app_server_stdio_command(task) => Some(CliCommand::ServerStdio),
        Some("server") => {
            if let Some(config) = a2a::parse(task)? {
                return Ok(CliCommand::ServerA2a(config));
            }
            let command = parse_app_server_http_command(task)?;
            match command {
                Some(config) => Some(CliCommand::ServerHttp(config)),
                None => bail!(
                    "usage: proteus [options] server stdio | server http [http-options] | server a2a [a2a-options]\n\
                     Global options must precede the command, for example: proteus --new-session server stdio"
                ),
            }
        }
        _ => Some(CliCommand::Task),
    };
    command.ok_or_else(|| anyhow::anyhow!("invalid command"))
}
