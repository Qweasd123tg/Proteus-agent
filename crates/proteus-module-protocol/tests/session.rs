use std::{
    env, fs,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use proteus_contracts::contracts::{
    PROCESS_COMPONENT_INITIALIZE_METHOD, PROCESS_COMPONENT_PROTOCOL_VERSION,
    PROCESS_MODULE_CANCEL_METHOD, PROCESS_SEARCH_CONTRACT_VERSION,
    PROCESS_WORKFLOW_CONTRACT_VERSION, ProcessComponentCall, ProcessComponentExportManifest,
    ProcessComponentExportRef, ProcessComponentInitialize, ProcessComponentManifest,
    ProcessModuleComposition, WORKFLOW_HOST_RUNTIME_STATUS_METHOD,
};
use proteus_module_protocol::{
    HostRequestDispatcher, ProcessComponentBinding, ProcessComponentSession,
    ProcessComponentSessionOptions, ProcessExportBinding, ProcessModuleHostRequest,
    ProcessModuleInvocationResult, ProcessModuleRpcError, ProcessModuleTerminal,
};
use proteus_process_host::{Framing, NewlineJsonFraming, ProcessSpec, ReceiveLimits};
use serde_json::{Value, json};

const SHORT_TIMEOUT: Duration = Duration::from_millis(500);
const CANCEL_GRACE: Duration = Duration::from_millis(75);

#[derive(Debug)]
struct TestSession {
    inner: ProcessComponentSession,
    target: ProcessComponentExportRef,
}

impl TestSession {
    fn invoke(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<ProcessModuleInvocationResult> {
        self.inner.invoke(&self.target, method, params, timeout)
    }

    fn invoke_with_cancel_check(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<ProcessModuleInvocationResult> {
        self.inner
            .invoke_with_cancel_check(&self.target, method, params, timeout, is_cancelled)
    }

    fn invoke_with_dispatcher_and_cancel_check(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        dispatcher: Arc<dyn HostRequestDispatcher>,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<ProcessModuleInvocationResult> {
        self.inner.invoke_with_dispatcher_and_cancel_check(
            &self.target,
            method,
            params,
            timeout,
            dispatcher,
            is_cancelled,
        )
    }
}

fn main() {
    let result = if env::args().nth(1).as_deref() == Some("__mock_module") {
        run_mock_module()
    } else {
        run_tests()
    };
    if let Err(error) = result {
        eprintln!("{error:?}");
        std::process::exit(1);
    }
}

type NamedTest = (&'static str, fn() -> Result<()>);

fn run_tests() -> Result<()> {
    let tests: &[NamedTest] = &[
        (
            "strict_initialize_and_success",
            strict_initialize_and_success,
        ),
        (
            "outgoing_method_is_contract_bounded",
            outgoing_method_is_contract_bounded,
        ),
        (
            "outgoing_target_is_component_bounded",
            outgoing_target_is_component_bounded,
        ),
        (
            "progress_is_bounded_to_known_methods",
            progress_is_bounded_to_known_methods,
        ),
        (
            "progress_retention_is_bounded",
            progress_retention_is_bounded,
        ),
        (
            "progress_retention_bytes_are_bounded",
            progress_retention_bytes_are_bounded,
        ),
        (
            "module_error_is_a_terminal_outcome",
            module_error_is_a_terminal_outcome,
        ),
        (
            "manifest_mismatch_fails_snapshot_build",
            manifest_mismatch_fails_snapshot_build,
        ),
        (
            "protocol_version_mismatch_fails_snapshot_build",
            protocol_version_mismatch_fails_snapshot_build,
        ),
        (
            "manifest_unknown_field_is_rejected",
            manifest_unknown_field_is_rejected,
        ),
        (
            "manifest_missing_export_is_rejected",
            manifest_missing_export_is_rejected,
        ),
        (
            "manifest_extra_export_is_rejected",
            manifest_extra_export_is_rejected,
        ),
        (
            "manifest_duplicate_export_is_rejected",
            manifest_duplicate_export_is_rejected,
        ),
        (
            "unoffered_feature_is_rejected",
            unoffered_feature_is_rejected,
        ),
        (
            "forbidden_host_callback_fails_closed",
            forbidden_host_callback_fails_closed,
        ),
        (
            "allowed_host_callback_uses_invocation_dispatcher",
            allowed_host_callback_uses_invocation_dispatcher,
        ),
        (
            "component_routes_multiple_exports",
            component_routes_multiple_exports,
        ),
        (
            "callback_authority_is_scoped_to_the_active_export",
            callback_authority_is_scoped_to_the_active_export,
        ),
        (
            "cancel_resets_the_whole_component_failure_domain",
            cancel_resets_the_whole_component_failure_domain,
        ),
        (
            "unknown_notification_poison_session",
            unknown_notification_poison_session,
        ),
        (
            "wrong_response_id_poison_session",
            wrong_response_id_poison_session,
        ),
        (
            "malformed_envelope_poison_session",
            malformed_envelope_poison_session,
        ),
        (
            "cooperative_cancel_has_canceled_terminal",
            cooperative_cancel_has_canceled_terminal,
        ),
        (
            "deadline_has_timed_out_terminal",
            deadline_has_timed_out_terminal,
        ),
        (
            "crash_is_followed_by_lazy_restart",
            crash_is_followed_by_lazy_restart,
        ),
    ];

    let mut failed = 0usize;
    for (name, test) in tests {
        print!("test {name} ... ");
        io::stdout().flush()?;
        match test() {
            Ok(()) => println!("ok"),
            Err(error) => {
                failed += 1;
                println!("FAILED");
                eprintln!("{name}: {error:?}");
            }
        }
    }
    if failed > 0 {
        bail!("{failed} process-module protocol test(s) failed");
    }
    Ok(())
}

fn strict_initialize_and_success() -> Result<()> {
    let session = connect("echo", &[])?;
    let result = session.invoke("search", json!({ "query": "needle" }), SHORT_TIMEOUT)?;

    assert_eq!(result.invocation_id, "invocation-1");
    assert_eq!(
        result.terminal,
        ProcessModuleTerminal::Success(json!({ "query": "needle" }))
    );
    assert!(result.notifications.is_empty());
    Ok(())
}

fn outgoing_method_is_contract_bounded() -> Result<()> {
    let session = connect("echo", &[])?;
    let error = session
        .invoke("module.private", json!({}), SHORT_TIMEOUT)
        .expect_err("host must not invoke a method outside the slot contract");

    ensure!(
        error.to_string().contains("is not part of search/v1"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn outgoing_target_is_component_bounded() -> Result<()> {
    let session = connect("echo", &[])?;
    let error = session
        .inner
        .invoke(
            &ProcessComponentExportRef::new("search", "not-bound"),
            "search",
            json!({}),
            SHORT_TIMEOUT,
        )
        .expect_err("host must not invoke an undeclared component export");

    ensure!(
        error
            .to_string()
            .contains("component \"fixture-component\" has no export search/not-bound"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn progress_is_bounded_to_known_methods() -> Result<()> {
    let session = connect("progress", &[])?;
    let result = session.invoke("search", json!({}), SHORT_TIMEOUT)?;

    assert_eq!(
        result.terminal,
        ProcessModuleTerminal::Success(json!({ "ok": true }))
    );
    assert_eq!(result.notifications.len(), 2);
    assert_eq!(result.notifications[0].method, "module.progress");
    assert_eq!(result.notifications[1].method, "module.activity");
    Ok(())
}

fn progress_retention_is_bounded() -> Result<()> {
    let session =
        connect_with_notification_limits("notification_burst", ReceiveLimits::new(2, 1024 * 1024))?;
    let error = session
        .invoke("search", json!({}), SHORT_TIMEOUT)
        .expect_err("third retained notification must exceed the bound");

    ensure!(
        format!("{error:#}").contains("notifications exceeded frame count limit"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn progress_retention_bytes_are_bounded() -> Result<()> {
    let session =
        connect_with_notification_limits("large_notification", ReceiveLimits::new(8, 100))?;
    let error = session
        .invoke("search", json!({}), SHORT_TIMEOUT)
        .expect_err("large retained notification must exceed the byte bound");

    ensure!(
        format!("{error:#}").contains("notifications exceeded byte limit"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn module_error_is_a_terminal_outcome() -> Result<()> {
    let session = connect("module_error", &[])?;
    let result = session.invoke("search", json!({}), SHORT_TIMEOUT)?;

    let ProcessModuleTerminal::ModuleError(error) = result.terminal else {
        bail!("expected module error terminal")
    };
    assert_eq!(error.code, 41);
    assert_eq!(error.message, "fixture failure");
    Ok(())
}

fn manifest_mismatch_fails_snapshot_build() -> Result<()> {
    let error = connect("wrong_composition", &[]).expect_err("composition mismatch must fail");
    ensure!(
        error.to_string().contains("handshake failed"),
        "unexpected error: {error:#}"
    );
    ensure!(
        format!("{error:#}").contains("composition mismatch"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn manifest_unknown_field_is_rejected() -> Result<()> {
    let error = connect("manifest_unknown", &[]).expect_err("unknown manifest field must fail");
    ensure!(
        format!("{error:#}").contains("invalid manifest"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn manifest_missing_export_is_rejected() -> Result<()> {
    let error = connect("manifest_missing", &[]).expect_err("missing export must fail");
    ensure!(
        format!("{error:#}").contains("omitted export search/fixture"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn manifest_extra_export_is_rejected() -> Result<()> {
    let error = connect("manifest_extra", &[]).expect_err("extra export must fail");
    ensure!(
        format!("{error:#}").contains("returned undeclared export search/extra"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn manifest_duplicate_export_is_rejected() -> Result<()> {
    let error = connect("manifest_duplicate", &[]).expect_err("duplicate export must fail");
    ensure!(
        format!("{error:#}").contains("manifest repeats export search/fixture"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn protocol_version_mismatch_fails_snapshot_build() -> Result<()> {
    let error = connect("wrong_protocol", &[]).expect_err("protocol mismatch must fail");
    ensure!(
        format!("{error:#}").contains("protocol mismatch"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn unoffered_feature_is_rejected() -> Result<()> {
    let error = connect("unoffered_feature", &[]).expect_err("unoffered feature must fail");
    ensure!(
        format!("{error:#}").contains("unoffered feature"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn forbidden_host_callback_fails_closed() -> Result<()> {
    let session = connect("forbidden_host", &[])?;
    let error = session
        .invoke("search", json!({}), SHORT_TIMEOUT)
        .expect_err("search has no callback authority");

    ensure!(
        format!("{error:#}").contains("forbidden host method"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

#[derive(Debug)]
struct RuntimeStatusDispatcher;

impl HostRequestDispatcher for RuntimeStatusDispatcher {
    fn dispatch(&self, request: ProcessModuleHostRequest) -> Result<Value, ProcessModuleRpcError> {
        assert_eq!(request.invocation_id, "invocation-1");
        assert_eq!(request.method, WORKFLOW_HOST_RUNTIME_STATUS_METHOD);
        assert_eq!(request.params, json!({}));
        Ok(json!({
            "cancelled": false,
            "queued_user_messages": 2,
        }))
    }
}

fn allowed_host_callback_uses_invocation_dispatcher() -> Result<()> {
    let session = connect_workflow("allowed_host")?;
    let result = session.invoke_with_dispatcher_and_cancel_check(
        "run",
        json!({}),
        SHORT_TIMEOUT,
        Arc::new(RuntimeStatusDispatcher),
        || false,
    )?;

    assert_eq!(
        result.terminal,
        ProcessModuleTerminal::Success(json!({
            "cancelled": false,
            "queued_user_messages": 2,
        }))
    );
    Ok(())
}

fn component_routes_multiple_exports() -> Result<()> {
    let (session, search, workflow) = connect_multi("multi_echo")?;

    let search_result =
        session.invoke(&search, "search", json!({"query": "needle"}), SHORT_TIMEOUT)?;
    assert_eq!(
        search_result.terminal,
        ProcessModuleTerminal::Success(json!({
            "export": {"slot": "search", "module_id": "fixture-search"},
            "params": {"query": "needle"},
        }))
    );

    let workflow_result =
        session.invoke(&workflow, "run", json!({"task": "continue"}), SHORT_TIMEOUT)?;
    assert_eq!(
        workflow_result.terminal,
        ProcessModuleTerminal::Success(json!({
            "export": {"slot": "workflow", "module_id": "fixture-workflow"},
            "params": {"task": "continue"},
        }))
    );
    Ok(())
}

#[derive(Debug)]
struct RestartedRuntimeStatusDispatcher;

impl HostRequestDispatcher for RestartedRuntimeStatusDispatcher {
    fn dispatch(&self, request: ProcessModuleHostRequest) -> Result<Value, ProcessModuleRpcError> {
        assert_eq!(request.invocation_id, "invocation-2");
        assert_eq!(request.method, WORKFLOW_HOST_RUNTIME_STATUS_METHOD);
        Ok(json!({
            "cancelled": false,
            "queued_user_messages": 3,
        }))
    }
}

fn callback_authority_is_scoped_to_the_active_export() -> Result<()> {
    let (session, search, workflow) = connect_multi("multi_authority")?;

    let error = session
        .invoke(&search, "search", json!({}), SHORT_TIMEOUT)
        .expect_err("search export must not inherit workflow callback authority");
    ensure!(
        format!("{error:#}")
            .contains("export search/fixture-search requested forbidden host method"),
        "unexpected error: {error:#}"
    );

    let workflow_result = session.invoke_with_dispatcher_and_cancel_check(
        &workflow,
        "run",
        json!({}),
        SHORT_TIMEOUT,
        Arc::new(RestartedRuntimeStatusDispatcher),
        || false,
    )?;
    assert_eq!(
        workflow_result.terminal,
        ProcessModuleTerminal::Success(json!({
            "cancelled": false,
            "queued_user_messages": 3,
        }))
    );
    Ok(())
}

fn cancel_resets_the_whole_component_failure_domain() -> Result<()> {
    let marker = unique_marker("component-cancel-reset");
    let _ = fs::remove_file(&marker);
    let marker_arg = marker.to_string_lossy().into_owned();
    let (session, search, workflow) =
        connect_multi_with_args("multi_cancel", &[marker_arg.as_str()])?;
    let canceled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&canceled);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        signal.store(true, Ordering::Release);
    });

    let search_result =
        session.invoke_with_cancel_check(&search, "search", json!({}), SHORT_TIMEOUT, || {
            canceled.load(Ordering::Acquire)
        })?;
    thread
        .join()
        .map_err(|_| anyhow!("cancel thread panicked"))?;
    assert_eq!(search_result.terminal, ProcessModuleTerminal::Canceled);
    let old_generation = search_result
        .notifications
        .first()
        .and_then(|notification| notification.params["generation"].as_u64())
        .ok_or_else(|| anyhow!("canceled export did not report its worker generation"))?;

    let workflow_result = session.invoke(&workflow, "run", json!({}), SHORT_TIMEOUT)?;
    let ProcessModuleTerminal::Success(result) = workflow_result.terminal else {
        bail!("second export did not succeed after component reset")
    };
    let new_generation = result["generation"]
        .as_u64()
        .ok_or_else(|| anyhow!("restarted component did not report its generation"))?;
    ensure!(
        (old_generation, new_generation) == (1, 2),
        "cancellation did not restart the component: {old_generation} -> {new_generation}"
    );
    let _ = fs::remove_file(marker);
    Ok(())
}

fn unknown_notification_poison_session() -> Result<()> {
    let session = connect("unknown_notification", &[])?;
    let error = session
        .invoke("search", json!({}), SHORT_TIMEOUT)
        .expect_err("unknown notification must fail");

    ensure!(
        format!("{error:#}").contains("unsupported notification"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn wrong_response_id_poison_session() -> Result<()> {
    let session = connect("wrong_id", &[])?;
    let error = session
        .invoke("search", json!({}), SHORT_TIMEOUT)
        .expect_err("wrong response id must fail");

    ensure!(
        format!("{error:#}").contains("did not match active invocation"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn malformed_envelope_poison_session() -> Result<()> {
    let session = connect("malformed_envelope", &[])?;
    let error = session
        .invoke("search", json!({}), SHORT_TIMEOUT)
        .expect_err("extra envelope field must fail");

    ensure!(
        format!("{error:#}").contains("envelope fields mismatch"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn cooperative_cancel_has_canceled_terminal() -> Result<()> {
    let session = connect("cooperative_cancel", &[])?;
    let canceled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&canceled);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        signal.store(true, Ordering::Release);
    });

    let result = session.invoke_with_cancel_check("search", json!({}), SHORT_TIMEOUT, || {
        canceled.load(Ordering::Acquire)
    })?;
    thread
        .join()
        .map_err(|_| anyhow!("cancel thread panicked"))?;

    assert_eq!(result.terminal, ProcessModuleTerminal::Canceled);
    Ok(())
}

fn deadline_has_timed_out_terminal() -> Result<()> {
    let session = connect("never_respond", &[])?;
    let result = session.invoke("search", json!({}), Duration::from_millis(40))?;

    assert_eq!(result.terminal, ProcessModuleTerminal::TimedOut);
    Ok(())
}

fn crash_is_followed_by_lazy_restart() -> Result<()> {
    let marker = unique_marker("lazy-restart");
    let session = connect(
        "exit_once",
        &[marker.as_os_str().to_string_lossy().as_ref()],
    )?;

    session
        .invoke("search", json!({}), SHORT_TIMEOUT)
        .expect_err("first worker must exit");
    let result = session.invoke("search", json!({ "after": "restart" }), SHORT_TIMEOUT)?;
    assert_eq!(
        result.terminal,
        ProcessModuleTerminal::Success(json!({ "after": "restart" }))
    );

    let _ = fs::remove_file(marker);
    Ok(())
}

fn connect(mode: &str, extra_args: &[&str]) -> Result<TestSession> {
    connect_with(mode, extra_args, ReceiveLimits::default())
}

fn connect_with_notification_limits(
    mode: &str,
    notification_limits: ReceiveLimits,
) -> Result<TestSession> {
    connect_with(mode, &[], notification_limits)
}

fn connect_with(
    mode: &str,
    extra_args: &[&str],
    notification_limits: ReceiveLimits,
) -> Result<TestSession> {
    let exe = env::current_exe()?;
    let exe = exe
        .to_str()
        .ok_or_else(|| anyhow!("test binary path is not UTF-8"))?;
    let mut args = vec!["__mock_module".to_owned(), mode.to_owned()];
    args.extend(extra_args.iter().map(|arg| (*arg).to_owned()));
    let export = ProcessExportBinding::new(
        "search",
        "fixture",
        PROCESS_SEARCH_CONTRACT_VERSION,
        json!({ "fixture": true }),
    )?;
    let target = export.export_ref();
    let binding = ProcessComponentBinding::new("fixture-component", [export])?;
    let inner = ProcessComponentSession::connect(
        ProcessSpec::new(exe).args(args),
        binding,
        ProcessComponentSessionOptions {
            handshake_timeout: SHORT_TIMEOUT,
            cancel_grace: CANCEL_GRACE,
            receive_limits: ReceiveLimits::default(),
            notification_limits,
        },
    )?;
    Ok(TestSession { inner, target })
}

fn connect_workflow(mode: &str) -> Result<TestSession> {
    let exe = env::current_exe()?;
    let exe = exe
        .to_str()
        .ok_or_else(|| anyhow!("test binary path is not UTF-8"))?;
    let export = ProcessExportBinding::new(
        "workflow",
        "fixture",
        PROCESS_WORKFLOW_CONTRACT_VERSION,
        json!({ "fixture": true }),
    )?;
    let target = export.export_ref();
    let binding = ProcessComponentBinding::new("fixture-component", [export])?;
    let inner = ProcessComponentSession::connect(
        ProcessSpec::new(exe).args(["__mock_module", mode]),
        binding,
        ProcessComponentSessionOptions {
            handshake_timeout: SHORT_TIMEOUT,
            cancel_grace: CANCEL_GRACE,
            receive_limits: ReceiveLimits::default(),
            notification_limits: ReceiveLimits::default(),
        },
    )?;
    Ok(TestSession { inner, target })
}

fn connect_multi(
    mode: &str,
) -> Result<(
    ProcessComponentSession,
    ProcessComponentExportRef,
    ProcessComponentExportRef,
)> {
    connect_multi_with_args(mode, &[])
}

fn connect_multi_with_args(
    mode: &str,
    extra_args: &[&str],
) -> Result<(
    ProcessComponentSession,
    ProcessComponentExportRef,
    ProcessComponentExportRef,
)> {
    let exe = env::current_exe()?;
    let exe = exe
        .to_str()
        .ok_or_else(|| anyhow!("test binary path is not UTF-8"))?;
    let search = ProcessExportBinding::new(
        "search",
        "fixture-search",
        PROCESS_SEARCH_CONTRACT_VERSION,
        json!({"fixture": "search"}),
    )?;
    let search_target = search.export_ref();
    let workflow = ProcessExportBinding::new(
        "workflow",
        "fixture-workflow",
        PROCESS_WORKFLOW_CONTRACT_VERSION,
        json!({"fixture": "workflow"}),
    )?;
    let workflow_target = workflow.export_ref();
    let binding = ProcessComponentBinding::new("fixture-component", [search, workflow])?;
    let mut args = vec!["__mock_module".to_owned(), mode.to_owned()];
    args.extend(extra_args.iter().map(|arg| (*arg).to_owned()));
    let session = ProcessComponentSession::connect(
        ProcessSpec::new(exe).args(args),
        binding,
        ProcessComponentSessionOptions {
            handshake_timeout: SHORT_TIMEOUT,
            cancel_grace: CANCEL_GRACE,
            receive_limits: ReceiveLimits::default(),
            notification_limits: ReceiveLimits::default(),
        },
    )?;
    Ok((session, search_target, workflow_target))
}

fn unique_marker(name: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "proteus-module-protocol-{name}-{}",
        std::process::id()
    ))
}

fn run_mock_module() -> Result<()> {
    let mode = env::args()
        .nth(2)
        .ok_or_else(|| anyhow!("missing mock module mode"))?;
    let marker = env::args().nth(3).map(PathBuf::from);
    if mode == "multi_cancel" {
        let marker = marker
            .as_deref()
            .ok_or_else(|| anyhow!("multi_cancel requires a startup marker"))?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(marker)?;
        writeln!(file, "started")?;
    }
    let framing = NewlineJsonFraming::default();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    initialize_mock(&framing, &mut reader, &mut writer, &mode)?;
    while let Ok(request) = framing.read_frame(&mut reader) {
        if request.get("id").is_none() {
            continue;
        }
        handle_invocation(
            &framing,
            &mut reader,
            &mut writer,
            &mode,
            marker.as_deref(),
            request,
        )?;
    }
    Ok(())
}

fn initialize_mock<R: BufRead, W: Write>(
    framing: &NewlineJsonFraming,
    reader: &mut R,
    writer: &mut W,
    mode: &str,
) -> Result<()> {
    let request = framing.read_frame(reader).context("missing initialize")?;
    ensure!(request["jsonrpc"] == "2.0");
    ensure!(request["method"] == PROCESS_COMPONENT_INITIALIZE_METHOD);
    let initialize: ProcessComponentInitialize =
        serde_json::from_value(request["params"].clone()).context("strict initialize params")?;
    assert_initialize(&initialize, mode)?;

    let mut manifest = serde_json::to_value(ProcessComponentManifest {
        protocol_version: PROCESS_COMPONENT_PROTOCOL_VERSION.to_owned(),
        component_id: initialize.component_id.clone(),
        exports: initialize
            .exports
            .iter()
            .map(|export| ProcessComponentExportManifest {
                slot: export.slot.clone(),
                module_id: export.module_id.clone(),
                contract_version: export.contract_version.clone(),
                composition: export.composition,
                module_features: Vec::new(),
            })
            .collect(),
    })?;
    match mode {
        "wrong_composition" => manifest["exports"][0]["composition"] = json!("ordered_many"),
        "wrong_protocol" => manifest["protocol_version"] = json!("v999"),
        "manifest_unknown" => {
            manifest["legacy"] = json!(true);
        }
        "manifest_missing" => manifest["exports"] = json!([]),
        "manifest_extra" => manifest["exports"]
            .as_array_mut()
            .expect("manifest exports")
            .push(json!({
                "slot": "search",
                "module_id": "extra",
                "contract_version": "v1",
                "composition": "select_one",
                "module_features": [],
            })),
        "manifest_duplicate" => {
            let duplicate = manifest["exports"][0].clone();
            manifest["exports"]
                .as_array_mut()
                .expect("manifest exports")
                .push(duplicate);
        }
        "unoffered_feature" => {
            manifest["exports"][0]["module_features"] = json!(["secret_feature"])
        }
        _ => {}
    }
    write_result(framing, writer, request["id"].clone(), manifest)
}

fn assert_initialize(initialize: &ProcessComponentInitialize, mode: &str) -> Result<()> {
    ensure!(initialize.protocol_version == PROCESS_COMPONENT_PROTOCOL_VERSION);
    ensure!(initialize.component_id == "fixture-component");
    if matches!(mode, "multi_echo" | "multi_authority" | "multi_cancel") {
        ensure!(initialize.exports.len() == 2);
        let search = &initialize.exports[0];
        ensure!(search.slot == "search");
        ensure!(search.module_id == "fixture-search");
        ensure!(search.contract_version == PROCESS_SEARCH_CONTRACT_VERSION);
        ensure!(search.module_config == json!({"fixture": "search"}));
        let workflow = &initialize.exports[1];
        ensure!(workflow.slot == "workflow");
        ensure!(workflow.module_id == "fixture-workflow");
        ensure!(workflow.contract_version == PROCESS_WORKFLOW_CONTRACT_VERSION);
        ensure!(workflow.module_config == json!({"fixture": "workflow"}));
        for export in &initialize.exports {
            ensure!(export.composition == ProcessModuleComposition::SelectOne);
            ensure!(export.host_features.is_empty());
        }
    } else {
        ensure!(initialize.exports.len() == 1);
        let export = &initialize.exports[0];
        let (slot, contract_version) = if mode == "allowed_host" {
            ("workflow", PROCESS_WORKFLOW_CONTRACT_VERSION)
        } else {
            ("search", PROCESS_SEARCH_CONTRACT_VERSION)
        };
        ensure!(export.slot == slot);
        ensure!(export.module_id == "fixture");
        ensure!(export.contract_version == contract_version);
        ensure!(export.composition == ProcessModuleComposition::SelectOne);
        ensure!(export.module_config == json!({ "fixture": true }));
        ensure!(export.host_features.is_empty());
    }
    Ok(())
}

fn handle_invocation<R: BufRead, W: Write>(
    framing: &NewlineJsonFraming,
    reader: &mut R,
    writer: &mut W,
    mode: &str,
    marker: Option<&Path>,
    request: Value,
) -> Result<()> {
    let id = request["id"].clone();
    let call: ProcessComponentCall =
        serde_json::from_value(request["params"].clone()).context("strict component call")?;
    match mode {
        "echo" => write_result(framing, writer, id, call.params),
        "multi_echo" => write_result(
            framing,
            writer,
            id,
            json!({"export": call.export, "params": call.params}),
        ),
        "multi_authority" => {
            framing.write_frame(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": "callback-1",
                    "method": WORKFLOW_HOST_RUNTIME_STATUS_METHOD,
                    "params": {},
                }),
            )?;
            let callback = framing.read_frame(reader)?;
            ensure!(callback["id"] == "callback-1");
            if call.export.slot == "workflow" {
                ensure!(callback.get("error").is_none());
                write_result(framing, writer, id, callback["result"].clone())
            } else {
                ensure!(callback.get("error").is_some());
                Ok(())
            }
        }
        "multi_cancel" if call.export.slot == "search" => {
            let generation = marker_generation(marker)?;
            write_notification(
                framing,
                writer,
                "module.activity",
                json!({"generation": generation}),
            )?;
            let cancel = framing
                .read_frame(reader)
                .context("host closed before component cancellation")?;
            ensure!(cancel["method"] == PROCESS_MODULE_CANCEL_METHOD);
            ensure!(cancel["params"]["id"] == id);
            write_error(framing, writer, id, -32800, "canceled")
        }
        "multi_cancel" => write_result(
            framing,
            writer,
            id,
            json!({"generation": marker_generation(marker)?}),
        ),
        "progress" => {
            write_notification(
                framing,
                writer,
                "module.progress",
                json!({ "completed": 1, "total": 2 }),
            )?;
            write_notification(
                framing,
                writer,
                "module.activity",
                json!({ "message": "fixture" }),
            )?;
            write_result(framing, writer, id, json!({ "ok": true }))
        }
        "notification_burst" => {
            for order in 1..=3 {
                write_notification(
                    framing,
                    writer,
                    "module.progress",
                    json!({ "order": order }),
                )?;
            }
            write_result(framing, writer, id, json!({ "ok": true }))
        }
        "large_notification" => {
            write_notification(
                framing,
                writer,
                "module.progress",
                json!({ "payload": "x".repeat(256) }),
            )?;
            write_result(framing, writer, id, json!({ "ok": true }))
        }
        "module_error" => write_error(framing, writer, id, 41, "fixture failure"),
        "forbidden_host" => {
            framing.write_frame(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": "host-1",
                    "method": "host.secret",
                    "params": {}
                }),
            )?;
            let _ = framing.read_frame(reader)?;
            Ok(())
        }
        "allowed_host" => {
            framing.write_frame(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": "callback-1",
                    "method": WORKFLOW_HOST_RUNTIME_STATUS_METHOD,
                    "params": {},
                }),
            )?;
            let callback = framing.read_frame(reader)?;
            ensure!(callback["id"] == "callback-1");
            ensure!(callback.get("error").is_none());
            write_result(framing, writer, id, callback["result"].clone())
        }
        "unknown_notification" => {
            write_notification(framing, writer, "module.private", json!({}))?;
            write_result(framing, writer, id, json!({ "unreachable": true }))
        }
        "wrong_id" => write_result(framing, writer, json!("other-invocation"), json!({})),
        "malformed_envelope" => framing.write_frame(
            writer,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {},
                "legacy": true
            }),
        ),
        "cooperative_cancel" => {
            let cancel = framing
                .read_frame(reader)
                .context("host closed before cancellation")?;
            ensure!(cancel["method"] == PROCESS_MODULE_CANCEL_METHOD);
            ensure!(cancel["params"]["id"] == id);
            write_error(framing, writer, id, -32800, "canceled")
        }
        "never_respond" => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
        "exit_once" => {
            let marker = marker.ok_or_else(|| anyhow!("exit_once requires marker"))?;
            if !marker.exists() {
                fs::write(marker, b"exited")?;
                std::process::exit(0);
            }
            write_result(framing, writer, id, call.params)
        }
        "wrong_composition" | "wrong_protocol" | "manifest_unknown" | "manifest_missing"
        | "manifest_extra" | "manifest_duplicate" | "unoffered_feature" => {
            bail!("host invoked a module after invalid handshake")
        }
        other => bail!("unknown mock module mode: {other}"),
    }
}

fn marker_generation(marker: Option<&Path>) -> Result<usize> {
    let marker = marker.ok_or_else(|| anyhow!("component mode requires a startup marker"))?;
    Ok(fs::read_to_string(marker)?.lines().count())
}

fn write_result<W: Write>(
    framing: &NewlineJsonFraming,
    writer: &mut W,
    id: Value,
    result: Value,
) -> Result<()> {
    framing.write_frame(
        writer,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    )
}

fn write_error<W: Write>(
    framing: &NewlineJsonFraming,
    writer: &mut W,
    id: Value,
    code: i64,
    message: &str,
) -> Result<()> {
    framing.write_frame(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }),
    )
}

fn write_notification<W: Write>(
    framing: &NewlineJsonFraming,
    writer: &mut W,
    method: &str,
    params: Value,
) -> Result<()> {
    framing.write_frame(
        writer,
        &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
    )
}
