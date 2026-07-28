use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering as AtomicOrdering},
};

use proteus_contracts::{
    abi_stable::sabi_trait::TD_Opaque,
    contracts::ToolInvocationOwner,
    domain::{new_session_id, new_thread_id, new_turn_id},
    plugin::{PluginToolHost, PluginToolHost_TO},
};
use serde_json::{Value, json};

use super::*;

struct TestToolHost {
    cancelled: Arc<AtomicBool>,
}

impl PluginToolHost for TestToolHost {
    fn is_cancelled(&self) -> RResult<bool, PluginToolError> {
        RResult::ROk(self.cancelled.load(AtomicOrdering::SeqCst))
    }
}

fn invocation_context(cwd: &std::path::Path) -> PluginToolInvocationContext {
    PluginToolInvocationContext {
        cwd: cwd.to_path_buf(),
        owner: ToolInvocationOwner::new(new_session_id(), new_thread_id(), new_turn_id()),
    }
}

fn with_host<T>(
    cancelled: Arc<AtomicBool>,
    invoke: impl FnOnce(&mut PluginToolHostMut<'_>) -> T,
) -> T {
    let mut host = TestToolHost { cancelled };
    let mut host_to: PluginToolHostMut<'_> = PluginToolHost_TO::from_ptr(&mut host, TD_Opaque);
    invoke(&mut host_to)
}

fn exec_command_with_context(context: &PluginToolInvocationContext, args: Value) -> Value {
    let call = json!({ "id": "call_exec", "name": "exec_command", "args": args });
    let context_json = serde_json::to_string(context).expect("context json");
    let result = with_host(Arc::new(AtomicBool::new(false)), |host| {
        exec_command_impl(&call.to_string(), &context_json, host)
    })
    .expect("invoke");
    serde_json::from_str(&result).expect("tool result")
}

fn exec_command(cwd: &std::path::Path, args: Value) -> Value {
    exec_command_with_context(&invocation_context(cwd), args)
}

fn write_stdin_result(context: &PluginToolInvocationContext, args: Value) -> Result<String> {
    let call = json!({ "id": "call_stdin", "name": "write_stdin", "args": args });
    let context_json = serde_json::to_string(context).expect("context json");
    with_host(Arc::new(AtomicBool::new(false)), |host| {
        write_stdin_impl(&call.to_string(), &context_json, host)
    })
}

fn write_stdin(context: &PluginToolInvocationContext, args: Value) -> Value {
    let result = write_stdin_result(context, args).expect("invoke");
    serde_json::from_str(&result).expect("tool result")
}

#[test]
fn exec_command_reports_exit_for_quick_command() {
    let dir = tempfile::tempdir().expect("workspace");

    let result = exec_command(dir.path(), json!({ "cmd": "printf marker42" }));

    assert_eq!(result["ok"], true);
    let output = result["output"].as_str().expect("output");
    assert!(output.contains("marker42"), "{output}");
    assert!(output.contains("Process exited with code 0"), "{output}");
    assert_eq!(result["metadata"]["exited"], true);
    assert_eq!(result["metadata"]["exit_code"], 0);
    assert_eq!(result["metadata"]["session_id"], Value::Null);
}

#[test]
fn exec_command_neutralizes_interactive_env() {
    let dir = tempfile::tempdir().expect("workspace");

    let result = exec_command(
        dir.path(),
        json!({ "cmd": "printf '%s|%s|%s' \"$TERM\" \"$GIT_PAGER\" \"$PAGER\"" }),
    );

    let output = result["output"].as_str().expect("output");
    assert!(output.contains("dumb|cat|cat"), "{output}");
}

#[test]
fn exec_command_keeps_session_and_write_stdin_interacts() {
    let dir = tempfile::tempdir().expect("workspace");
    let context = invocation_context(dir.path());

    let started =
        exec_command_with_context(&context, json!({ "cmd": "cat", "yield_time_ms": 300 }));
    assert_eq!(started["ok"], true);
    assert_eq!(started["metadata"]["exited"], false);
    let session_id = started["metadata"]["session_id"]
        .as_i64()
        .expect("session id");
    assert!(
        started["output"]
            .as_str()
            .expect("output")
            .contains(&format!("Process running with session ID {session_id}"))
    );

    let echoed = write_stdin(
        &context,
        json!({
            "session_id": session_id,
            "chars": "hello\n",
            "yield_time_ms": 500
        }),
    );
    assert_eq!(echoed["ok"], true);
    assert!(
        echoed["output"].as_str().expect("output").contains("hello"),
        "{echoed}"
    );

    let finished = write_stdin(
        &context,
        json!({
            "session_id": session_id,
            "chars": "\u{4}",
            "yield_time_ms": 5000
        }),
    );
    assert_eq!(finished["metadata"]["exited"], true, "{finished}");
    assert_eq!(finished["metadata"]["exit_code"], 0);

    let error = write_stdin_result(&context, json!({ "session_id": session_id, "chars": "x" }))
        .expect_err("session must be gone");
    assert!(
        error.to_string().contains("unknown exec session"),
        "{error}"
    );
}

#[test]
fn janitor_keeps_recently_exited_session_until_output_is_collected() {
    let dir = tempfile::tempdir().expect("workspace");
    let context = invocation_context(dir.path());
    let started = exec_command_with_context(
        &context,
        json!({
            "cmd": "sleep 0.5; printf late-tail; exit 7",
            "yield_time_ms": 250
        }),
    );
    assert_eq!(started["metadata"]["exited"], false, "{started}");
    let session_id = started["metadata"]["session_id"]
        .as_i64()
        .expect("session id");

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let exited = lock(sessions())
            .get(&session_id)
            .is_some_and(|session| lock(&session.output).exited);
        if exited {
            break;
        }
        assert!(Instant::now() < deadline, "process did not exit in time");
        std::thread::sleep(Duration::from_millis(10));
    }

    prune_expired_sessions(Instant::now(), SESSION_MAX_IDLE);
    let collected = write_stdin(
        &context,
        json!({ "session_id": session_id, "yield_time_ms": 1000 }),
    );

    assert_eq!(collected["metadata"]["exited"], true, "{collected}");
    assert_eq!(collected["metadata"]["exit_code"], 7, "{collected}");
    assert!(
        collected["output"]
            .as_str()
            .expect("output")
            .contains("late-tail"),
        "{collected}"
    );
}

#[test]
fn write_stdin_enforces_session_thread_and_workspace_ownership() {
    let dir = tempfile::tempdir().expect("workspace");
    let owner_context = invocation_context(dir.path());
    let started = exec_command_with_context(
        &owner_context,
        json!({ "cmd": "cat", "yield_time_ms": 300 }),
    );
    let session_id = started["metadata"]["session_id"]
        .as_i64()
        .expect("session id");

    let mut foreign_context = owner_context.clone();
    foreign_context.owner.session_id = new_session_id();
    let error = write_stdin_result(
        &foreign_context,
        json!({ "session_id": session_id, "chars": "foreign\n" }),
    )
    .expect_err("foreign session must not control the PTY");
    assert!(error.to_string().contains("is not owned"), "{error}");

    let mut next_turn_context = owner_context.clone();
    next_turn_context.owner.turn_id = new_turn_id();
    let finished = write_stdin(
        &next_turn_context,
        json!({
            "session_id": session_id,
            "chars": "\u{4}",
            "yield_time_ms": 5000
        }),
    );
    assert_eq!(finished["metadata"]["exited"], true, "{finished}");
}

#[test]
fn cancellation_kills_and_removes_interactive_session() {
    let dir = tempfile::tempdir().expect("workspace");
    let context = invocation_context(dir.path());
    let context_json = serde_json::to_string(&context).expect("context json");
    let call = json!({
        "id": "call_cancel",
        "name": "exec_command",
        "args": {
            "cmd": "cat",
            "yield_time_ms": 30000
        }
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel_from_thread = cancelled.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        cancel_from_thread.store(true, AtomicOrdering::SeqCst);
    });

    let started = Instant::now();
    let error = with_host(cancelled, |host| {
        exec_command_impl(&call.to_string(), &context_json, host)
    })
    .expect_err("cancelled invocation must fail");
    canceller.join().expect("canceller");

    assert!(error.to_string().contains("canceled"), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancellation was not observed promptly"
    );
    assert!(
        lock(sessions())
            .values()
            .all(|session| !session.owner.matches(&context)),
        "cancelled session must be removed from the registry"
    );
}

#[cfg(unix)]
#[test]
fn session_owner_compares_canonical_workspace_paths() {
    let dir = tempfile::tempdir().expect("workspace root");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    let alias = dir.path().join("workspace-link");
    std::os::unix::fs::symlink(&workspace, &alias).expect("workspace symlink");

    let alias_context = invocation_context(&alias);
    let owner = ExecSessionOwner::from_context(
        &alias_context,
        workspace
            .canonicalize()
            .expect("canonical workspace")
            .to_string_lossy()
            .as_ref(),
    );
    let mut canonical_context = alias_context;
    canonical_context.cwd = workspace;

    assert!(owner.matches(&canonical_context));
}

#[test]
fn write_stdin_rejects_unknown_session() {
    let dir = tempfile::tempdir().expect("workspace");
    let context = invocation_context(dir.path());

    let error = write_stdin_result(&context, json!({ "session_id": -1 }))
        .expect_err("unknown session must error");

    assert!(
        error.to_string().contains("unknown exec session"),
        "{error}"
    );
}

#[test]
fn exec_command_requires_cmd_arg() {
    let dir = tempfile::tempdir().expect("workspace");
    let call = json!({ "id": "call_exec", "name": "exec_command", "args": {} });
    let context_json =
        serde_json::to_string(&invocation_context(dir.path())).expect("context json");

    let error = with_host(Arc::new(AtomicBool::new(false)), |host| {
        exec_command_impl(&call.to_string(), &context_json, host)
    })
    .expect_err("missing cmd must error");

    assert!(error.to_string().contains("requires string arg 'cmd'"));
}

#[test]
fn exec_command_sandbox_mode_fails_closed_without_bwrap() {
    let dir = tempfile::tempdir().expect("workspace");
    let marker = dir.path().join("must-not-exist");
    let args = json!({ "cmd": "touch must-not-exist" });
    let context = invocation_context(dir.path());

    let error = with_host(Arc::new(AtomicBool::new(false)), |host| {
        execute_command(
            "call_exec".to_owned(),
            Some(&args),
            args["cmd"].as_str().unwrap(),
            &context,
            SandboxMode::enabled_unavailable_for_test(),
            host,
        )
    })
    .expect_err("sandbox mode must fail closed without bwrap");

    assert!(error.to_string().contains("PROTEUS_SHELL_SANDBOX=1"));
    assert!(error.to_string().contains("bwrap"));
    assert!(!marker.exists(), "command must not be spawned");
}

#[test]
fn exec_command_trusted_mode_allows_external_workdir() {
    let workspace = tempfile::tempdir().expect("workspace");
    let external = tempfile::tempdir().expect("external workdir");
    let args = json!({ "cmd": "pwd", "workdir": external.path() });
    let context = invocation_context(workspace.path());

    let result = with_host(Arc::new(AtomicBool::new(false)), |host| {
        execute_command(
            "call_exec".to_owned(),
            Some(&args),
            "pwd",
            &context,
            SandboxMode::disabled_for_test(),
            host,
        )
    })
    .map(|json| serde_json::from_str::<Value>(&json).expect("tool result"))
    .expect("trusted external workdir");

    assert_eq!(result["ok"], true);
    assert_eq!(
        result["metadata"]["workdir"],
        external.path().display().to_string()
    );
    assert_eq!(result["metadata"]["sandbox"], Value::Null);
}

#[test]
fn exec_command_sandbox_mode_rejects_external_workdir() {
    let workspace = tempfile::tempdir().expect("workspace");
    let external = tempfile::tempdir().expect("external workdir");
    let args = json!({ "cmd": "pwd", "workdir": external.path() });
    let context = invocation_context(workspace.path());

    let error = with_host(Arc::new(AtomicBool::new(false)), |host| {
        execute_command(
            "call_exec".to_owned(),
            Some(&args),
            "pwd",
            &context,
            SandboxMode::enabled_unavailable_for_test(),
            host,
        )
    })
    .expect_err("sandbox mode must reject external workdir");

    assert!(
        error.to_string().contains("outside the workspace"),
        "{error}"
    );
    assert!(error.to_string().contains("PROTEUS_SHELL_SANDBOX=1"));
}

#[test]
fn exec_command_truncates_output_head_and_tail() {
    let dir = tempfile::tempdir().expect("workspace");

    let result = exec_command(
        dir.path(),
        json!({
            "cmd": "yes 0123456789 | head -c 60000",
            "max_output_tokens": 100
        }),
    );

    assert_eq!(result["metadata"]["truncated"], true);
    let output = result["output"].as_str().expect("output");
    assert!(output.contains("omitted"), "{output}");
}

#[test]
fn exec_command_reports_nonzero_exit_as_data_not_tool_failure() {
    let dir = tempfile::tempdir().expect("workspace");

    let result = exec_command(dir.path(), json!({ "cmd": "exit 7" }));

    assert_eq!(result["ok"], true);
    assert_eq!(result["error"], Value::Null);
    assert_eq!(result["metadata"]["exit_code"], 7);
    let output = result["output"].as_str().expect("output");
    assert!(output.contains("Process exited with code 7"), "{output}");
}

#[test]
fn exec_command_clamps_yield_time() {
    let dir = tempfile::tempdir().expect("workspace");
    let context = invocation_context(dir.path());

    let result =
        exec_command_with_context(&context, json!({ "cmd": "sleep 3", "yield_time_ms": 1 }));

    assert_eq!(result["metadata"]["yield_time_ms"], MIN_YIELD_MS);
    assert_eq!(result["metadata"]["exited"], false);
    let session_id = result["metadata"]["session_id"]
        .as_i64()
        .expect("session id");
    write_stdin(
        &context,
        json!({ "session_id": session_id, "chars": "\u{3}", "yield_time_ms": 5000 }),
    );
}

#[test]
fn prune_prefers_exited_then_oldest() {
    let now = Instant::now();
    let older = now - Duration::from_secs(60);
    assert_eq!(
        session_to_prune(&[(1, older, false), (2, now, false)]),
        Some(1)
    );
    assert_eq!(
        session_to_prune(&[(1, older, false), (2, now, true)]),
        Some(2)
    );
    assert_eq!(session_to_prune(&[]), None);
}

#[test]
fn age_cleanup_selects_only_idle_sessions() {
    let now = Instant::now();
    let old = now - SESSION_MAX_IDLE - Duration::from_secs(1);
    let fresh = now - Duration::from_secs(1);
    let mut expired = expired_session_ids(
        &[
            (1, old, false),
            (2, fresh, false),
            (3, fresh, true),
            (4, old, true),
        ],
        now,
        SESSION_MAX_IDLE,
    );
    expired.sort_unstable();

    assert_eq!(expired, vec![1, 4]);
}

#[test]
fn truncate_head_tail_respects_char_boundaries() {
    let text = "ёжик".repeat(100);
    let (truncated, was_truncated) = truncate_head_tail(&text, 101);
    assert!(was_truncated);
    assert!(truncated.contains("omitted"));

    let (untouched, was_truncated) = truncate_head_tail("short", 100);
    assert!(!was_truncated);
    assert_eq!(untouched, "short");
}
