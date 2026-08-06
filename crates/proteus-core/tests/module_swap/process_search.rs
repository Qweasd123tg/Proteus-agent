use super::*;

fn rg_available() -> bool {
    std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn fixture_path() -> std::path::PathBuf {
    workspace_root_file("crates/proteus-core/tests/fixtures/process_search.sh")
}

fn reference_path() -> std::path::PathBuf {
    workspace_root_file("examples/modules/search-process/search.py")
}

fn process_search_config(command: &str, args: Vec<String>, module_id: &str) -> AppConfig {
    let mut config = test_config();
    config.modules.search = "process".to_owned();
    set_module_config(
        &mut config,
        "search",
        "process",
        json!({
            "module_id": module_id,
            "command": command,
            "args": args,
            "timeout_ms": 3000
        }),
    );
    config
}

fn fixture_config(mode: &str) -> AppConfig {
    process_search_config(
        "sh",
        vec![fixture_path().display().to_string(), mode.to_owned()],
        "fixture",
    )
}

fn build_registry(config: &AppConfig, cwd: &std::path::Path) -> anyhow::Result<RuntimeRegistry> {
    RuntimeRegistry::from_catalog(config, cwd.to_path_buf(), test_catalog())
}

#[tokio::test]
async fn search_slot_swaps_null_and_process_implementations() {
    let dir = temp_workspace();
    let null = registry_from_test_config(&test_config(), dir.path());
    let null_chunks = null
        .search
        .search(SearchQuery::new("needle", dir.path().to_path_buf(), 5))
        .await
        .expect("null search");
    assert!(null_chunks.is_empty());

    let config = fixture_config("static");
    let process = build_registry(&config, dir.path()).expect("process registry");
    let chunks = process
        .search
        .search(SearchQuery::new("needle", dir.path().to_path_buf(), 5))
        .await
        .expect("process search");

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].source, "process:fixture");
    assert_eq!(chunks[0].content, "hit needle");
}

#[test]
fn handshake_mismatch_is_a_snapshot_build_error() {
    let dir = temp_workspace();
    let config = fixture_config("mismatch");

    let error = match build_registry(&config, dir.path()) {
        Ok(_) => panic!("mismatched process module must not build"),
        Err(error) => error,
    };

    let message = format!("{error:#}");
    assert!(message.contains("handshake failed"), "{message}");
    assert!(message.contains("slot mismatch"), "{message}");
}

#[tokio::test]
async fn process_failure_is_returned_without_null_fallback() {
    let dir = temp_workspace();
    let config = fixture_config("error");
    let registry = build_registry(&config, dir.path()).expect("process registry");

    let error = registry
        .search
        .search(SearchQuery::new("needle", dir.path().to_path_buf(), 5))
        .await
        .expect_err("process error must propagate");

    let message = format!("{error:#}");
    assert!(message.contains("request failed"), "{message}");
    assert!(message.contains("fixture search failure"), "{message}");
}

#[tokio::test]
async fn process_death_is_returned_without_null_fallback() {
    let dir = temp_workspace();
    let config = fixture_config("exit");
    let registry = build_registry(&config, dir.path()).expect("process registry");

    let error = registry
        .search
        .search(SearchQuery::new("needle", dir.path().to_path_buf(), 5))
        .await
        .expect_err("dead child must fail the selected slot");

    assert!(error.to_string().contains("request failed"), "{error:#}");
}

#[tokio::test]
async fn invalid_slot_response_is_rejected_without_legacy_array_support() {
    let dir = temp_workspace();
    let config = fixture_config("invalid");
    let registry = build_registry(&config, dir.path()).expect("process registry");

    let error = registry
        .search
        .search(SearchQuery::new("needle", dir.path().to_path_buf(), 5))
        .await
        .expect_err("bare array response must fail");

    assert!(
        error.to_string().contains("returned invalid response"),
        "{error:#}"
    );
}

#[tokio::test]
async fn reference_python_module_searches_with_rg() {
    if !rg_available()
        || !std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    {
        return;
    }
    let dir = temp_workspace();
    let config = process_search_config(
        "python3",
        vec![reference_path().display().to_string()],
        "python_rg",
    );
    let registry = build_registry(&config, dir.path()).expect("reference process registry");

    let chunks = registry
        .search
        .search(SearchQuery::new(
            "modular agent",
            dir.path().to_path_buf(),
            5,
        ))
        .await
        .expect("reference process search");

    assert!(chunks.iter().any(|chunk| {
        chunk.source == "process:python_rg"
            && chunk.path.as_deref() == Some(std::path::Path::new("sample.txt"))
            && chunk.content.contains("hello modular agent")
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn async_runtime_build_keeps_tokio_worker_responsive_during_handshake() {
    let dir = temp_workspace();
    let config = fixture_config("slow_initialize");
    let started = std::time::Instant::now();

    let build = AgentRuntime::builder(config, dir.path().to_path_buf())
        .with_module_catalog(test_catalog())
        .build_async();
    let heartbeat = async {
        tokio::time::sleep(Duration::from_millis(40)).await;
        started.elapsed()
    };
    let (runtime, heartbeat_elapsed) = tokio::join!(build, heartbeat);

    assert!(
        heartbeat_elapsed < Duration::from_millis(250),
        "process handshake blocked the Tokio worker for {heartbeat_elapsed:?}"
    );
    drop(runtime.expect("runtime"));
}
