//! Boundary evidence for root-coordinated messaging between full Proteus
//! peers connected through the local stdio process backend.

use std::{path::PathBuf, sync::Arc};

use proteus_core::{
    contracts::{
        AgentAddress, AgentControl, AgentControlMessage, AgentControlRequest, AgentLifecycleStatus,
        ApprovalPolicy, CancellationToken, EventEmitter, ExecutionScope, PolicyContext,
        PolicyVisibilityContext, ToolRegistry,
    },
    core::{
        AgentControlConfig, AgentControlRuntime, HeadlessApprovalTransport,
        HeadlessUserInputTransport, InMemoryEventStore,
    },
    domain::{
        AgentTask, ModelRef, PolicyDecision, ReasoningConfig, ToolCall, new_session_id,
        new_thread_id, new_turn_id,
    },
    stubs::{
        EmptyContextBuilder, FakeModelClient, NoCompactor, NoMemory, NullPatchApplier, NullSearch,
        UnfilteredToolExposure,
    },
};
use serde_json::json;

struct AllowAllPolicy;

impl ApprovalPolicy for AllowAllPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        PolicyDecision::Allow
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

fn test_runtime_context() -> proteus_core::contracts::RuntimeContext {
    proteus_core::contracts::RuntimeContext::new(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        ExecutionScope::fresh(CancellationToken::new()),
        ModelRef::new("fake", "fake-tool-model"),
        ReasoningConfig::default(),
        120_000,
        30_000,
        Arc::new(EventEmitter::new(Arc::new(InMemoryEventStore::new()))),
        Arc::new(FakeModelClient::default()),
        Arc::new(NullSearch),
        Arc::new(NoMemory),
        Arc::new(EmptyContextBuilder),
        ToolRegistry::new(),
        Arc::new(AllowAllPolicy),
        Arc::new(HeadlessApprovalTransport),
        Arc::new(HeadlessUserInputTransport),
        Arc::new(NullPatchApplier),
        Arc::new(NoCompactor),
        Arc::new(UnfilteredToolExposure),
        None,
    )
}

/// Full child Proteus with an external Python workflow and delayed streaming
/// fake model. The delay creates a deterministic in-flight window without a
/// network dependency or a test-only production hook.
fn write_messaging_child_config(config_home: &std::path::Path) -> PathBuf {
    let configs_dir = config_home.join("configs");
    std::fs::create_dir_all(&configs_dir).expect("create configs dir");
    let config_path = configs_dir.join("messaging.toml");
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/modules/agent-worker/agent.py")
        .canonicalize()
        .expect("canonical agent worker path");
    let worker = serde_json::to_string(&worker.to_string_lossy()).expect("quote worker path");
    std::fs::write(
        &config_path,
        format!(
            r#"active_provider = "fake"

[providers.fake]
provider = "fake"
model = "fake-tool-model"
stream = true

[providers.fake.provider_config]
stream_delay_ms = 40

[modules]
workflow = "python_agent_loop"

[components.python-agent]
command = "python3"
args = ["-B", {worker}]
handshake_timeout_ms = 30000

[components.python-agent.exports.workflow.python_agent_loop]

[module_config.workflow.python_agent_loop]
max_tool_rounds = 2
system_instructions = "Process-agent mailbox regression fixture."

[tools]
enabled = []
"#
        ),
    )
    .expect("write messaging child config");
    config_path
}

fn agent_message(target: &str, content: &str) -> AgentControlMessage {
    AgentControlMessage::from_root(AgentAddress::child(target).expect("agent address"), content)
        .expect("agent message")
}

fn collaboration_request(
    role: &str,
    target: &str,
    prompt: &str,
    task: AgentTask,
) -> AgentControlRequest {
    AgentControlRequest::new(role, prompt, task).with_metadata(json!({
        "control_plane_owned": true,
        "agent_control_target": AgentAddress::child(target)
            .expect("agent target")
            .as_str(),
    }))
}

fn messaging_runner(config_path: &std::path::Path) -> Arc<dyn AgentControl> {
    runner_from_json(json!({
        "binary": proteus_binary(),
        "max_parallel": 2,
        "roles": [
            {
                "name": "helper",
                "description": "Message-capable peer",
                "config": config_path.to_string_lossy(),
                "parallel_safe": true,
                "max_processes": 2,
                "timeout_ms": 60000
            }
        ]
    }))
}

fn proteus_binary() -> PathBuf {
    std::env::var_os("PROTEUS_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_proteus")))
}

fn runner_from_json(value: serde_json::Value) -> Arc<dyn AgentControl> {
    let config: AgentControlConfig = serde_json::from_value(value).expect("agent control config");
    AgentControlRuntime::from_config(&config)
        .expect("build agent control runtime")
        .service()
        .expect("configured agent control service")
}

#[cfg(unix)]
fn write_terminal_race_peer(root: &std::path::Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let marker = root.join("initial-request-received");
    let script = root.join("terminal-race-peer.py");
    let marker_literal =
        serde_json::to_string(&marker.to_string_lossy()).expect("quote marker path");
    std::fs::write(
        &script,
        format!(
            r#"#!/usr/bin/env python3
import json
import pathlib
import sys
import time

marker = pathlib.Path({marker_literal})
first = True
for line in sys.stdin:
    request = json.loads(line)
    if request.get("type") != "send":
        continue
    if first:
        first = False
        marker.write_text("ready", encoding="utf-8")
        time.sleep(0.25)
        text = "initial-result"
    else:
        text = request["text"]
    print(json.dumps({{
        "type": "response",
        "id": request.get("id"),
        "ok": True,
        "output": {{"text": text, "metadata": None}},
        "error": None,
    }}), flush=True)
"#
        ),
    )
    .expect("write terminal-race peer");
    let mut permissions = std::fs::metadata(&script)
        .expect("terminal-race peer metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("make terminal-race peer executable");
    (script, marker)
}

#[tokio::test]
async fn process_agents_route_bounded_messages_without_cross_delivery() {
    let config_home = tempfile::tempdir().expect("config home");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = write_messaging_child_config(config_home.path());
    let runner = messaging_runner(&config_path);
    assert_eq!(runner.profiles().len(), 1);

    let ctx = test_runtime_context();
    let task = AgentTask::new("delegate", workspace.path().to_path_buf());
    let alpha = runner
        .spawn(
            collaboration_request("helper", "alpha", "initial alpha", task.clone()),
            ctx.clone(),
        )
        .await
        .expect("spawn alpha");
    let beta = runner
        .spawn(
            collaboration_request("helper", "beta", "initial beta", task),
            ctx,
        )
        .await
        .expect("spawn beta");

    let wrong_target = runner
        .send(&alpha, agent_message("beta", "forged-target"))
        .await
        .expect_err("handle target must be exact");
    assert!(
        wrong_target
            .to_string()
            .contains("does not match handle target"),
        "{wrong_target:#}"
    );
    let mut forged_source = agent_message("alpha", "forged-source");
    forged_source.source = AgentAddress::child("beta").expect("forged source");
    let wrong_source = runner
        .send(&alpha, forged_source)
        .await
        .expect_err("v1 source must remain root-owned");
    assert!(
        wrong_source.to_string().contains("source must be /root"),
        "{wrong_source:#}"
    );

    runner
        .send(&alpha, agent_message("alpha", "alpha-only-payload"))
        .await
        .expect("message alpha");
    runner
        .send(&alpha, agent_message("alpha", "alpha-second-payload"))
        .await
        .expect("second FIFO message alpha");
    runner
        .send(&beta, agent_message("beta", "beta-only-payload"))
        .await
        .expect("message beta");

    let (alpha, beta) = tokio::join!(runner.wait(&alpha), runner.wait(&beta));
    let alpha = alpha.expect("alpha result");
    let beta = beta.expect("beta result");
    assert_eq!(alpha.status, AgentLifecycleStatus::Completed);
    assert_eq!(beta.status, AgentLifecycleStatus::Completed);
    assert!(
        alpha.summary.contains("alpha-second-payload"),
        "{}",
        alpha.summary
    );
    assert!(
        !alpha.summary.contains("beta-only-payload"),
        "{}",
        alpha.summary
    );
    assert!(
        beta.summary.contains("beta-only-payload"),
        "{}",
        beta.summary
    );
    assert!(
        !beta.summary.contains("alpha-only-payload"),
        "{}",
        beta.summary
    );
    assert_eq!(alpha.metadata["agent_messages_delivered"], 2);
    assert_eq!(beta.metadata["agent_messages_delivered"], 1);

    let unowned = runner
        .spawn(
            AgentControlRequest::new(
                "helper",
                "not addressable",
                AgentTask::new("delegate", workspace.path().to_path_buf()),
            ),
            test_runtime_context(),
        )
        .await
        .expect("spawn non-control-plane child");
    let unowned_error = runner
        .send(&unowned, agent_message("alpha", "must-not-route"))
        .await
        .expect_err("unbound handle must reject addressed delivery");
    assert!(
        unowned_error
            .to_string()
            .contains("not owned by the collaboration control plane"),
        "{unowned_error:#}"
    );
    runner.cancel(&unowned).await.expect("cancel unowned child");
    assert_eq!(
        runner.wait(&unowned).await.expect("unowned result").status,
        AgentLifecycleStatus::Cancelled
    );
}

#[cfg(unix)]
#[tokio::test]
async fn process_agent_terminal_race_returns_the_message_started_turn() {
    let fixture = tempfile::tempdir().expect("fixture");
    let workspace = tempfile::tempdir().expect("workspace");
    let (binary, marker) = write_terminal_race_peer(fixture.path());
    let runner = runner_from_json(json!({
        "binary": binary,
        "max_parallel": 1,
        "max_idle_processes": 0,
        "roles": [{
            "name": "helper",
            "description": "Protocol race peer",
            "config": "ignored-by-fixture",
            "parallel_safe": true,
            "max_processes": 1,
            "timeout_ms": 10000
        }]
    }));
    let handle = runner
        .spawn(
            collaboration_request(
                "helper",
                "race",
                "initial",
                AgentTask::new("delegate", workspace.path().to_path_buf()),
            ),
            test_runtime_context(),
        )
        .await
        .expect("spawn terminal-race peer");

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !marker.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("peer accepted initial request");
    runner
        .send(&handle, agent_message("race", "terminal-race-payload"))
        .await
        .expect("queue message during terminal race");

    let result = runner.wait(&handle).await.expect("terminal-race result");
    assert_eq!(result.status, AgentLifecycleStatus::Completed);
    assert!(
        result.summary.contains("terminal-race-payload"),
        "message-started turn must supersede the initial result: {}",
        result.summary
    );
    assert_eq!(result.metadata["agent_messages_delivered"], 1);
}

#[tokio::test]
async fn process_agent_cancel_is_targeted_and_closes_only_its_mailbox() {
    let config_home = tempfile::tempdir().expect("config home");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = write_messaging_child_config(config_home.path());
    let runner = messaging_runner(&config_path);
    let ctx = test_runtime_context();
    let task = AgentTask::new("delegate", workspace.path().to_path_buf());

    let cancelled = runner
        .spawn(
            collaboration_request("helper", "cancelled", "cancel me", task.clone()),
            ctx.clone(),
        )
        .await
        .expect("spawn cancelled peer");
    let survivor = runner
        .spawn(
            collaboration_request("helper", "survivor", "keep working", task),
            ctx,
        )
        .await
        .expect("spawn survivor");
    runner
        .send(
            &survivor,
            agent_message("survivor", "survivor-after-cancel"),
        )
        .await
        .expect("message survivor");
    runner.cancel(&cancelled).await.expect("targeted cancel");
    assert!(
        runner
            .send(&cancelled, agent_message("cancelled", "late-message"))
            .await
            .is_err(),
        "cancel closes only the target mailbox"
    );

    let (cancelled, survivor) = tokio::join!(runner.wait(&cancelled), runner.wait(&survivor));
    assert_eq!(
        cancelled.expect("cancelled result").status,
        AgentLifecycleStatus::Cancelled
    );
    let survivor = survivor.expect("survivor result");
    assert_eq!(survivor.status, AgentLifecycleStatus::Completed);
    assert!(survivor.summary.contains("survivor-after-cancel"));
}

#[tokio::test]
async fn process_agent_startup_crash_does_not_fail_a_live_sibling() {
    let config_home = tempfile::tempdir().expect("config home");
    let workspace = tempfile::tempdir().expect("workspace");
    let good_config = write_messaging_child_config(config_home.path());
    let missing_config = config_home.path().join("missing-child.toml");
    let runner = runner_from_json(json!({
        "binary": proteus_binary(),
        "max_parallel": 2,
        "roles": [
            {
                "name": "good",
                "description": "Healthy peer",
                "config": good_config.to_string_lossy(),
                "parallel_safe": true,
                "max_processes": 1,
                "timeout_ms": 60000
            },
            {
                "name": "broken",
                "description": "Broken peer",
                "config": missing_config.to_string_lossy(),
                "parallel_safe": true,
                "max_processes": 1,
                "timeout_ms": 60000
            }
        ]
    }));
    let ctx = test_runtime_context();
    let task = AgentTask::new("delegate", workspace.path().to_path_buf());

    let broken = runner
        .spawn(
            AgentControlRequest::new("broken", "fail", task.clone()),
            ctx.clone(),
        )
        .await
        .expect("spawn broken peer process");
    let good = runner
        .spawn(collaboration_request("good", "good", "work", task), ctx)
        .await
        .expect("spawn good peer");
    runner
        .send(&good, agent_message("good", "healthy-sibling-result"))
        .await
        .expect("message good peer");

    let (broken, good) = tokio::join!(runner.wait(&broken), runner.wait(&good));
    let broken = broken.expect_err("broken child must fail independently");
    assert!(
        format!("{broken:#}").contains("exited unexpectedly"),
        "{broken:#}"
    );
    let good = good.expect("healthy sibling result");
    assert_eq!(good.status, AgentLifecycleStatus::Completed);
    assert!(good.summary.contains("healthy-sibling-result"));
}
