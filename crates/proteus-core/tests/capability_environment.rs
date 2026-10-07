use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Result, ensure};
use proteus_contracts::{
    contracts::{
        ExecutionAttribution, PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION,
        PROCESS_CONTEXT_PROVIDER_METHOD, PROCESS_TOOL_CONTRACT_VERSION, PROCESS_TOOL_INVOKE_METHOD,
        ProcessContextChunksResponse, ProcessContextProviderInput, ProcessToolInvokeInput,
        ProcessToolInvokeResponse,
    },
    domain::{AgentTask, ToolCall, new_call_id, new_execution_id},
};
use proteus_core::core::AppConfig;
use proteus_module_protocol::{
    ProcessComponentBinding, ProcessExportBinding,
    v3::{ComponentBroker, ComponentBrokerOptions, InvocationTerminal},
};
use proteus_process_host::{NewlineJsonFraming, ProcessTransport};
use serde_json::{Value, json};

const MODE: &str = "PROTEUS_SKILL_ENV_MODE";
const WORKSPACE: &str = "PROTEUS_SKILL_ENV_WORKSPACE";
const UNRELATED: &str = "PROTEUS_SKILL_ENV_UNRELATED";
const TEST_TIMEOUT: Duration = Duration::from_secs(3);

#[tokio::test]
async fn supplied_capabilities_discover_user_skills_and_preserve_environment_isolation()
-> Result<()> {
    if std::env::var_os(MODE).is_some() {
        return worker_scenario().await;
    }
    // Environment mutation stays in isolated test processes, never in the
    // concurrently running test harness or the owner's actual HOME.
    for mode in ["home", "proteus_home"] {
        let state = tempfile::tempdir()?;
        let home = state.path().join("home");
        let proteus_home = state.path().join("custom-proteus");
        let workspace = state.path().join("workspace");
        fs::create_dir_all(&workspace)?;
        // Discovery uses the nearest repository root. Keep this fixture from
        // inheriting a repository marker belonging to an ancestor of /tmp.
        fs::create_dir(workspace.join(".git"))?;
        let root = if mode == "home" {
            home.join(".proteus")
        } else {
            proteus_home.clone()
        };
        write_skill(&root.join("skills/installed"), "User body.")?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "supplied_capabilities_discover_user_skills_and_preserve_environment_isolation",
                "--nocapture",
            ])
            .env(MODE, mode)
            .env(WORKSPACE, &workspace)
            .env(UNRELATED, "must-not-leak")
            .env("HOME", &home)
            .env_remove("PROTEUS_HOME");
        if mode == "proteus_home" {
            command.env("PROTEUS_HOME", &proteus_home);
        }
        let mut child = command.spawn()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(
                    status.success(),
                    "isolated {mode} capabilities scenario failed"
                );
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                anyhow::bail!("isolated {mode} capabilities scenario timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}

fn write_skill(directory: &Path, body: &str) -> Result<()> {
    fs::create_dir_all(directory)?;
    fs::write(
        directory.join("SKILL.md"),
        format!("---\nname: installed\ndescription: Installed test skill\n---\n{body}\n"),
    )?;
    Ok(())
}

async fn worker_scenario() -> Result<()> {
    let cwd = PathBuf::from(std::env::var_os(WORKSPACE).expect("isolated workspace"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let profile = AppConfig::load(Some(&root.join("configs/config.toml"))).await?;
    let mut spec = profile.components["reference-capabilities"].process_spec(&cwd)?;
    spec.command = std::env::var("PROTEUS_TEST_REFERENCE_MODULE")?;
    spec.args.clear();

    // Exercise the same launch description through an independent environment
    // probe. The component grants only its configured roots plus launch defaults.
    let mut probe = spec.clone();
    probe.command = "python3".into();
    probe.args = vec![
        "-B".into(),
        "-c".into(),
        "import json,os,sys; print(json.dumps(dict(os.environ)), flush=True); sys.stdin.readline()"
            .into(),
    ];
    let mut transport = ProcessTransport::spawn(&probe, NewlineJsonFraming::default())?;
    let environment = transport.recv_frame(TEST_TIMEOUT)?;
    ensure!(environment.get(UNRELATED).is_none() && environment.get(MODE).is_none());
    ensure!(environment["HOME"] == json!(std::env::var("HOME")?));
    if std::env::var(MODE)? == "proteus_home" {
        ensure!(environment["PROTEUS_HOME"] == json!(std::env::var("PROTEUS_HOME")?));
    }
    transport.terminate()?;

    let provider = ProcessExportBinding::new(
        "context_provider",
        "skills",
        PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION,
        json!({}),
    )?;
    let tool = ProcessExportBinding::new(
        "tool",
        "reference.tools",
        PROCESS_TOOL_CONTRACT_VERSION,
        json!({}),
    )?;
    let provider_ref = provider.export_ref();
    let tool_ref = tool.export_ref();
    let broker = ComponentBroker::connect(
        spec,
        ProcessComponentBinding::new("reference-capabilities", [provider, tool])?,
        ComponentBrokerOptions::default(),
    )?;
    for (source, body) in [("user", "User body."), ("project", "Project override.")] {
        if source == "project" {
            write_skill(&cwd.join(".proteus/skills/installed"), body)?;
        }
        let input = ProcessContextProviderInput {
            provider_id: "skills".into(),
            task: AgentTask::new("discover installed skill", cwd.clone()),
            metadata: Value::Null,
        };
        let catalog = broker
            .invoke(
                &provider_ref,
                PROCESS_CONTEXT_PROVIDER_METHOD,
                serde_json::to_value(
                    proteus_contracts::contracts::ProcessContextProviderRequest {
                        input,
                        skills: Default::default(),
                    },
                )?,
                TEST_TIMEOUT,
            )
            .await?;
        let InvocationTerminal::Success(catalog) = catalog else {
            anyhow::bail!("skills catalogue failed: {catalog:?}");
        };
        let catalog: ProcessContextChunksResponse = serde_json::from_value(catalog)?;
        ensure!(
            catalog
                .result
                .iter()
                .any(|chunk| chunk.content.contains("<name>installed</name>"))
        );
        let input = ProcessToolInvokeInput {
            skills: Default::default(),
            call: ToolCall::new(new_call_id(), "skill", json!({"name":"installed"})),
            cwd: cwd.clone(),
            attribution: ExecutionAttribution::detached(new_execution_id()),
        };
        let terminal = broker
            .invoke(
                &tool_ref,
                PROCESS_TOOL_INVOKE_METHOD,
                serde_json::to_value(input)?,
                TEST_TIMEOUT,
            )
            .await?;
        let InvocationTerminal::Success(result) = terminal else {
            anyhow::bail!("skill invocation failed: {terminal:?}");
        };
        let result: ProcessToolInvokeResponse = serde_json::from_value(result)?;
        ensure!(
            result.result.ok && result.result.output.trim() == body,
            "unexpected {source} skill result: {:?}",
            result.result
        );
        ensure!(result.result.metadata["source"] == source);
    }
    Ok(())
}
