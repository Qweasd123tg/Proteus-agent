use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use clap::Parser;
use proteus_contracts::contracts::{PROCESS_COMPONENT_PROTOCOL_VERSION, ProcessComponentExportRef};
use proteus_module_protocol::{
    ProcessComponentBinding, ProcessExportBinding,
    v3::{ComponentBroker, ComponentBrokerOptions, InvocationTerminal},
};
use proteus_process_host::ProcessSpec;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Parser)]
#[command(
    name = "proteus-component-conformance",
    about = "Run the strict Proteus process-component v3 handshake and an optional export probe"
)]
struct Cli {
    #[arg(long)]
    component_id: String,
    /// Export binding as strict JSON: {"slot":"search","module_id":"rg","contract_version":"v1","module_config":{}}.
    #[arg(long = "export", required = true)]
    exports: Vec<String>,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "env-allowlist")]
    env_allowlist: Vec<String>,
    #[arg(long = "env", value_parser = parse_env)]
    env: Vec<(String, String)>,
    /// Probe target in `slot/module_id` form.
    #[arg(long)]
    probe_export: Option<String>,
    #[arg(long)]
    probe_method: Option<String>,
    #[arg(long)]
    probe_params: Option<String>,
    /// Worker command and arguments. Must follow `--`.
    #[arg(required = true, last = true, num_args = 1..)]
    command: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportArgument {
    slot: String,
    module_id: String,
    contract_version: String,
    #[serde(default = "empty_object")]
    module_config: Value,
}

fn empty_object() -> Value {
    json!({})
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
    if cli.probe_method.is_some() != cli.probe_export.is_some() {
        bail!("--probe-method and --probe-export must be supplied together");
    }
    if cli.probe_params.is_some() && cli.probe_method.is_none() {
        bail!("--probe-params requires --probe-method");
    }

    let exports = cli
        .exports
        .iter()
        .map(|value| {
            let export: ExportArgument = serde_json::from_str(value)
                .with_context(|| format!("--export must be strict JSON: {value}"))?;
            ProcessExportBinding::new(
                export.slot,
                export.module_id,
                export.contract_version,
                export.module_config,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let binding = ProcessComponentBinding::new(cli.component_id.clone(), exports)?;

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

    let timeout = Duration::from_millis(cli.timeout_ms);
    let broker = ComponentBroker::connect(
        spec,
        binding.clone(),
        ComponentBrokerOptions {
            handshake_timeout: timeout,
            ..ComponentBrokerOptions::default()
        },
    )?;
    let exports = binding
        .exports
        .iter()
        .map(|export| {
            let authority = export.authority()?;
            Ok(json!({
                "slot": export.slot,
                "module_id": export.module_id,
                "contract_version": export.contract_version,
                "composition": authority.composition,
                "module_methods": authority.module_methods,
                "host_methods": authority.host_methods,
                "host_features": authority.host_features,
                "required_features": authority.required_features,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut report = json!({
        "protocol_version": PROCESS_COMPONENT_PROTOCOL_VERSION,
        "component_id": cli.component_id,
        "exports": exports,
        "handshake": "success"
    });

    if let (Some(target), Some(method)) = (cli.probe_export, cli.probe_method) {
        let target = parse_export_ref(&target)?;
        let params = parse_json(
            "--probe-params",
            cli.probe_params.as_deref().unwrap_or("{}"),
        )?;
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let (invocation_id, terminal, notifications) = runtime.block_on(async {
            let mut handle = broker
                .start_invocation(&target, &method, params, timeout)
                .await?;
            let invocation_id = handle.id().to_owned();
            let mut notifications = handle.notifications()?;
            let terminal = handle.result().await?;
            let mut collected = Vec::new();
            while let Ok(notification) = notifications.try_recv() {
                collected.push(notification);
            }
            Ok::<_, anyhow::Error>((invocation_id, terminal, collected))
        })?;
        let output = match terminal {
            InvocationTerminal::Success(output) => output,
            InvocationTerminal::ModuleError(error) => {
                return Err(error).context("conformance probe returned a module error");
            }
            InvocationTerminal::Canceled => bail!("conformance probe was canceled"),
            InvocationTerminal::TimedOut => bail!("conformance probe timed out"),
            InvocationTerminal::ComponentLost(failure) => {
                bail!("conformance probe lost its component: {failure:?}")
            }
        };
        let notifications = notifications
            .into_iter()
            .map(|notification| {
                json!({
                    "method": notification.method,
                    "params": notification.params
                })
            })
            .collect::<Vec<_>>();
        report["probe"] = json!({
            "export": target,
            "method": method,
            "invocation_id": invocation_id,
            "terminal": "success",
            "notifications": notifications,
            "result": output
        });
    }

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn parse_export_ref(value: &str) -> Result<ProcessComponentExportRef> {
    let Some((slot, module_id)) = value.split_once('/') else {
        bail!("--probe-export must use slot/module_id form");
    };
    if slot.trim().is_empty() || module_id.trim().is_empty() {
        bail!("--probe-export slot and module id must not be empty");
    }
    Ok(ProcessComponentExportRef::new(slot, module_id))
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
    fn export_ref_parser_is_strict() {
        assert_eq!(
            parse_export_ref("search/rg").expect("target"),
            ProcessComponentExportRef::new("search", "rg")
        );
        parse_export_ref("search").expect_err("missing module id must fail");
    }

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
