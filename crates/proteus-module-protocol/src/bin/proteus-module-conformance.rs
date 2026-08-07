use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use clap::Parser;
use proteus_module_protocol::{
    ProcessModuleBinding, ProcessModuleSession, ProcessModuleSessionOptions, ProcessModuleTerminal,
};
use proteus_process_host::ProcessSpec;
use serde_json::{Value, json};

#[derive(Debug, Parser)]
#[command(
    name = "proteus-module-conformance",
    about = "Run the strict Proteus process-module v1 handshake and an optional contract probe"
)]
struct Cli {
    #[arg(long)]
    slot: String,
    #[arg(long)]
    module_id: String,
    #[arg(long)]
    contract_version: String,
    #[arg(long, default_value = "{}")]
    module_config: String,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "env-allowlist")]
    env_allowlist: Vec<String>,
    #[arg(long = "env", value_parser = parse_env)]
    env: Vec<(String, String)>,
    #[arg(long)]
    probe_method: Option<String>,
    #[arg(long, requires = "probe_method")]
    probe_params: Option<String>,
    /// Worker command and arguments. Must follow `--`.
    #[arg(required = true, last = true, num_args = 1..)]
    command: Vec<String>,
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    if cli.timeout_ms == 0 {
        bail!("--timeout-ms must be greater than zero");
    }
    let module_config = parse_json("--module-config", &cli.module_config)?;
    let (command, args) = cli
        .command
        .split_first()
        .expect("clap requires a worker command");
    let mut spec = ProcessSpec::new(command.clone())
        .args(args.iter().cloned())
        .env_allowlist(cli.env_allowlist)
        .envs(cli.env);
    if let Some(cwd) = cli.cwd {
        spec = spec.cwd(cwd);
    }

    let binding = ProcessModuleBinding::new(
        cli.slot.clone(),
        cli.module_id.clone(),
        cli.contract_version.clone(),
        module_config,
    )?;
    let timeout = Duration::from_millis(cli.timeout_ms);
    let session = ProcessModuleSession::connect(
        spec,
        binding,
        ProcessModuleSessionOptions {
            handshake_timeout: timeout,
            ..ProcessModuleSessionOptions::default()
        },
    )?;
    let authority = session.authority();
    let mut report = json!({
        "protocol_version": "v1",
        "slot": cli.slot,
        "module_id": cli.module_id,
        "contract_version": cli.contract_version,
        "composition": authority.composition,
        "module_methods": authority.module_methods,
        "host_methods": authority.host_methods,
        "host_features": authority.host_features,
        "required_features": authority.required_features,
        "handshake": "success"
    });

    if let Some(method) = cli.probe_method {
        let params = parse_json(
            "--probe-params",
            cli.probe_params.as_deref().unwrap_or("{}"),
        )?;
        let invocation = session.invoke(&method, params, timeout)?;
        let output = match invocation.terminal {
            ProcessModuleTerminal::Success(output) => output,
            ProcessModuleTerminal::ModuleError(error) => {
                return Err(error).context("conformance probe returned a module error");
            }
            ProcessModuleTerminal::Canceled => bail!("conformance probe was canceled"),
            ProcessModuleTerminal::TimedOut => bail!("conformance probe timed out"),
        };
        let notifications = invocation
            .notifications
            .into_iter()
            .map(|notification| {
                json!({
                    "method": notification.method,
                    "params": notification.params
                })
            })
            .collect::<Vec<_>>();
        report["probe"] = json!({
            "method": method,
            "invocation_id": invocation.invocation_id,
            "terminal": "success",
            "notifications": notifications,
            "result": output
        });
    }

    session.terminate()?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn parse_json(argument: &str, value: &str) -> Result<Value> {
    serde_json::from_str(value).with_context(|| format!("{argument} must be valid JSON"))
}

fn parse_env(value: &str) -> std::result::Result<(String, String), String> {
    let Some((name, value)) = value.split_once('=') else {
        return Err("expected NAME=VALUE".to_owned());
    };
    if name.is_empty() {
        return Err("environment name must not be empty".to_owned());
    }
    Ok((name.to_owned(), value.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn env_parser_preserves_equals_in_value() {
        assert_eq!(
            parse_env("TOKEN=a=b").expect("environment pair"),
            ("TOKEN".to_owned(), "a=b".to_owned())
        );
    }

    #[test]
    fn duplicate_explicit_env_uses_last_value() {
        let values = vec![
            parse_env("TOKEN=first").expect("first"),
            parse_env("TOKEN=second").expect("second"),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>();

        assert_eq!(values["TOKEN"], "second");
    }
}
