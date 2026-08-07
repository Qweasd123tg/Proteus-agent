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
    PROCESS_MODULE_CANCEL_METHOD, PROCESS_MODULE_INITIALIZE_METHOD,
    PROCESS_MODULE_PROTOCOL_VERSION, PROCESS_SEARCH_CONTRACT_VERSION,
    PROCESS_WORKFLOW_CONTRACT_VERSION, ProcessModuleComposition, ProcessModuleInitialize,
    ProcessModuleManifest, WORKFLOW_HOST_RUNTIME_STATUS_METHOD,
};
use proteus_module_protocol::{
    HostRequestDispatcher, ProcessModuleBinding, ProcessModuleHostRequest, ProcessModuleRpcError,
    ProcessModuleSession, ProcessModuleSessionOptions, ProcessModuleTerminal,
};
use proteus_process_host::{Framing, NewlineJsonFraming, ProcessSpec, ReceiveLimits};
use serde_json::{Value, json};

const SHORT_TIMEOUT: Duration = Duration::from_millis(500);
const CANCEL_GRACE: Duration = Duration::from_millis(75);

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

fn connect(mode: &str, extra_args: &[&str]) -> Result<ProcessModuleSession> {
    connect_with(mode, extra_args, ReceiveLimits::default())
}

fn connect_with_notification_limits(
    mode: &str,
    notification_limits: ReceiveLimits,
) -> Result<ProcessModuleSession> {
    connect_with(mode, &[], notification_limits)
}

fn connect_with(
    mode: &str,
    extra_args: &[&str],
    notification_limits: ReceiveLimits,
) -> Result<ProcessModuleSession> {
    let exe = env::current_exe()?;
    let exe = exe
        .to_str()
        .ok_or_else(|| anyhow!("test binary path is not UTF-8"))?;
    let mut args = vec!["__mock_module".to_owned(), mode.to_owned()];
    args.extend(extra_args.iter().map(|arg| (*arg).to_owned()));
    let binding = ProcessModuleBinding::new(
        "search",
        "fixture",
        PROCESS_SEARCH_CONTRACT_VERSION,
        json!({ "fixture": true }),
    )?;
    ProcessModuleSession::connect(
        ProcessSpec::new(exe).args(args),
        binding,
        ProcessModuleSessionOptions {
            handshake_timeout: SHORT_TIMEOUT,
            cancel_grace: CANCEL_GRACE,
            receive_limits: ReceiveLimits::default(),
            notification_limits,
        },
    )
}

fn connect_workflow(mode: &str) -> Result<ProcessModuleSession> {
    let exe = env::current_exe()?;
    let exe = exe
        .to_str()
        .ok_or_else(|| anyhow!("test binary path is not UTF-8"))?;
    let binding = ProcessModuleBinding::new(
        "workflow",
        "fixture",
        PROCESS_WORKFLOW_CONTRACT_VERSION,
        json!({ "fixture": true }),
    )?;
    ProcessModuleSession::connect(
        ProcessSpec::new(exe).args(["__mock_module", mode]),
        binding,
        ProcessModuleSessionOptions {
            handshake_timeout: SHORT_TIMEOUT,
            cancel_grace: CANCEL_GRACE,
            receive_limits: ReceiveLimits::default(),
            notification_limits: ReceiveLimits::default(),
        },
    )
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
    ensure!(request["method"] == PROCESS_MODULE_INITIALIZE_METHOD);
    let initialize: ProcessModuleInitialize =
        serde_json::from_value(request["params"].clone()).context("strict initialize params")?;
    let (slot, contract_version) = if mode == "allowed_host" {
        ("workflow", PROCESS_WORKFLOW_CONTRACT_VERSION)
    } else {
        ("search", PROCESS_SEARCH_CONTRACT_VERSION)
    };
    assert_initialize(&initialize, slot, contract_version)?;

    let mut manifest = serde_json::to_value(ProcessModuleManifest {
        protocol_version: PROCESS_MODULE_PROTOCOL_VERSION.to_owned(),
        slot: slot.to_owned(),
        module_id: "fixture".to_owned(),
        contract_version: contract_version.to_owned(),
        composition: ProcessModuleComposition::SelectOne,
        module_features: Vec::new(),
    })?;
    match mode {
        "wrong_composition" => manifest["composition"] = json!("ordered_many"),
        "wrong_protocol" => manifest["protocol_version"] = json!("v999"),
        "manifest_unknown" => {
            manifest["legacy"] = json!(true);
        }
        "unoffered_feature" => manifest["module_features"] = json!(["secret_feature"]),
        _ => {}
    }
    write_result(framing, writer, request["id"].clone(), manifest)
}

fn assert_initialize(
    initialize: &ProcessModuleInitialize,
    slot: &str,
    contract_version: &str,
) -> Result<()> {
    ensure!(initialize.protocol_version == PROCESS_MODULE_PROTOCOL_VERSION);
    ensure!(initialize.slot == slot);
    ensure!(initialize.module_id == "fixture");
    ensure!(initialize.contract_version == contract_version);
    ensure!(initialize.composition == ProcessModuleComposition::SelectOne);
    ensure!(initialize.module_config == json!({ "fixture": true }));
    ensure!(initialize.host_features.is_empty());
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
    match mode {
        "echo" => write_result(framing, writer, id, request["params"].clone()),
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
            write_result(framing, writer, id, request["params"].clone())
        }
        "wrong_composition" | "wrong_protocol" | "manifest_unknown" | "unoffered_feature" => {
            bail!("host invoked a module after invalid handshake")
        }
        other => bail!("unknown mock module mode: {other}"),
    }
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
