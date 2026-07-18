use super::*;

fn python_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn rg_available() -> bool {
    std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn fixture_path() -> std::path::PathBuf {
    workspace_root_file("crates/proteus-core/tests/fixtures/process_search.py")
}

fn reference_path() -> std::path::PathBuf {
    workspace_root_file("examples/modules/search-process/search.py")
}

fn process_search_config(script: std::path::PathBuf, args: &[&str], module_id: &str) -> AppConfig {
    let mut config = test_config();
    config.modules.search = "process".to_owned();
    let mut command_args = vec![script.display().to_string()];
    command_args.extend(args.iter().map(|arg| (*arg).to_owned()));
    set_module_config(
        &mut config,
        "search",
        "process",
        json!({
            "module_id": module_id,
            "command": "python3",
            "args": command_args,
            "timeout_ms": 3000
        }),
    );
    config
}

fn build_registry(config: &AppConfig, cwd: &std::path::Path) -> anyhow::Result<BuiltinRegistry> {
    BuiltinRegistry::from_catalog(config, cwd.to_path_buf(), test_catalog())
}

#[tokio::test]
async fn search_slot_swaps_null_and_process_implementations() {
    if !python_available() {
        return;
    }
    let dir = temp_workspace();
    let null = registry_from_test_config(&test_config(), dir.path());
    let null_chunks = null
        .search
        .search(SearchQuery::new("needle", dir.path().to_path_buf(), 5))
        .await
        .expect("null search");
    assert!(null_chunks.is_empty());

    let config = process_search_config(fixture_path(), &["static"], "fixture");
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
    if !python_available() {
        return;
    }
    let dir = temp_workspace();
    let config = process_search_config(fixture_path(), &["mismatch"], "fixture");

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
    if !python_available() {
        return;
    }
    let dir = temp_workspace();
    let config = process_search_config(fixture_path(), &["error"], "fixture");
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
    if !python_available() {
        return;
    }
    let dir = temp_workspace();
    let config = process_search_config(fixture_path(), &["exit"], "fixture");
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
    if !python_available() {
        return;
    }
    let dir = temp_workspace();
    let config = process_search_config(fixture_path(), &["invalid"], "fixture");
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
    if !python_available() || !rg_available() {
        return;
    }
    let dir = temp_workspace();
    let config = process_search_config(reference_path(), &[], "python_rg");
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
