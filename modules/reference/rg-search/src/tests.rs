use super::*;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn parse_rg_match_extracts_path_line_and_content() {
    let chunk = parse_rg_match(r#"{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"let value = 1;\n"},"line_number":42}}"#).unwrap();

    assert_eq!(chunk.source, "rg");
    assert_eq!(chunk.path.unwrap().display().to_string(), "src/main.rs");
    assert_eq!(chunk.content, "let value = 1;");
    assert_eq!(chunk.metadata["line"], 42);
}

#[test]
fn parse_rg_match_normalizes_current_dir_prefix() {
    let chunk = parse_rg_match(r#"{"type":"match","data":{"path":{"text":"./src/main.rs"},"lines":{"text":"let value = 1;\n"},"line_number":42}}"#).unwrap();

    assert_eq!(chunk.path.unwrap().display().to_string(), "src/main.rs");
}

#[test]
fn rg_command_searches_workspace_path_explicitly() {
    let query = SearchQuery::new("needle", std::path::PathBuf::from("/tmp/workspace"), 10);
    let command = build_rg_command(&query);
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(args.last().map(String::as_str), Some("."));
    assert_eq!(
        command.get_current_dir(),
        Some(std::path::Path::new("/tmp/workspace"))
    );
}

#[test]
fn rg_command_uses_safe_path_filters_as_search_roots_and_globs() {
    let dir = temp_workspace();
    fs::create_dir(dir.join("src")).unwrap();
    let query = SearchQuery::new("needle", dir.clone(), 10)
        .with_path_filters(["src/", "../outside", "/tmp"], [".rs", "../secret"]);
    let command = build_rg_command(&query);
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert!(!args.iter().any(|arg| arg == "--glob"));
    assert_eq!(args.last().map(String::as_str), Some("src"));
    assert!(!args.iter().any(|arg| arg == "../outside"));
    assert!(!args.iter().any(|arg| arg == "/tmp"));
    assert!(!args.iter().any(|arg| arg.contains("secret")));
    let plain = build_rg_command(
        &SearchQuery::new("needle", dir.clone(), 10).with_path_filters(["src/"], [".rs"]),
    );
    let plain_args = plain
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(plain_args.windows(2).any(|pair| pair == ["--glob", "*.rs"]));
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn literal_path_filters_match_python_component() {
    use proteus_contracts::contracts::{
        ExecutionAttribution, PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION,
        PROCESS_CONTEXT_PROVIDER_METHOD, ProcessContextChunksResponse, ProcessContextProviderInput,
        ProcessContextProviderRequest,
    };
    use proteus_module_protocol::{
        ProcessComponentBinding, ProcessExportBinding,
        v3::{ComponentBroker, ComponentBrokerOptions, InvocationTerminal},
    };
    use proteus_process_host::ProcessSpec;

    let dir = temp_workspace();
    for path in [
        "routes/[id].ts",
        "routes/id.ts",
        "nested/src/alpha.rs",
        "src/alpha.rs",
        "src/also.rs",
        "src-other/a.rs",
        "docs/question?.txt",
        "docs/questionx.txt",
        "docs/{one,two}.txt",
        "docs/one.txt",
    ] {
        let target = dir.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, "needle\n").unwrap();
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/modules/search-process/search.py");
    let binding = ProcessExportBinding::new(
        "context_provider",
        "python_rg",
        PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION,
        json!({}),
    )
    .unwrap();
    let target = binding.export_ref();
    let broker = ComponentBroker::connect(
        ProcessSpec::new("python3").args(["-B".to_owned(), script.display().to_string()]),
        ProcessComponentBinding::new("python-search-swap", [binding]).unwrap(),
        ComponentBrokerOptions {
            handshake_timeout: Duration::from_secs(2),
            ..ComponentBrokerOptions::default()
        },
    )
    .unwrap();
    for (prefixes, suffixes, expected) in [
        (vec![], vec!["[id].ts"], vec!["routes/[id].ts"]),
        (
            vec![],
            vec![".rs", "[id].ts"],
            vec![
                "nested/src/alpha.rs",
                "routes/[id].ts",
                "src-other/a.rs",
                "src/alpha.rs",
                "src/also.rs",
            ],
        ),
        (vec![], vec!["question?.txt"], vec!["docs/question?.txt"]),
        (
            vec![],
            vec!["src/alpha.rs"],
            vec!["nested/src/alpha.rs", "src/alpha.rs"],
        ),
        (vec![], vec!["{one,two}.txt"], vec!["docs/{one,two}.txt"]),
        (vec!["src/a"], vec![], vec!["src/alpha.rs", "src/also.rs"]),
        (vec!["./src/"], vec![], vec!["src/alpha.rs", "src/also.rs"]),
        (
            vec!["src"],
            vec![],
            vec!["src-other/a.rs", "src/alpha.rs", "src/also.rs"],
        ),
        (vec!["missing/"], vec![], vec![]),
    ] {
        let query =
            SearchQuery::new("needle", dir.clone(), 20).with_path_filters(prefixes, suffixes);
        let mut rust_paths = run_rg(query.clone())
            .unwrap()
            .into_iter()
            .map(|chunk| chunk.path.unwrap().display().to_string())
            .collect::<Vec<_>>();
        rust_paths.sort();
        assert_eq!(rust_paths, expected, "Rust query: {query:?}");
        let terminal = broker
            .invoke_blocking(
                &target,
                PROCESS_CONTEXT_PROVIDER_METHOD,
                serde_json::to_value(ProcessContextProviderRequest {
                    input: ProcessContextProviderInput {
                        provider_id: "python_rg".into(),
                        task: proteus_contracts::domain::AgentTask::new("search", dir.clone()),
                        metadata: serde_json::to_value(&query).unwrap(),
                    },
                    attribution: ExecutionAttribution::detached(
                        proteus_contracts::domain::new_execution_id(),
                    ),
                    skills: Default::default(),
                })
                .unwrap(),
                Duration::from_secs(2),
            )
            .unwrap();
        let InvocationTerminal::Success(value) = terminal else {
            panic!("Python search failed: {terminal:?}");
        };
        let response: ProcessContextChunksResponse = serde_json::from_value(value).unwrap();
        let mut python_paths = response
            .result
            .into_iter()
            .map(|chunk| chunk.path.unwrap().display().to_string())
            .collect::<Vec<_>>();
        python_paths.sort();
        assert_eq!(python_paths, rust_paths, "Python query: {query:?}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn run_rg_returns_matches_from_tiny_workspace() {
    let dir = temp_workspace();
    fs::write(dir.join("a.txt"), "hello needle\n").expect("write a.txt");
    fs::write(dir.join("b.txt"), "other\nneedle two\n").expect("write b.txt");

    let chunks = run_rg(SearchQuery::new("needle", dir.clone(), 10)).expect("rg search");
    let paths = chunks
        .iter()
        .map(|chunk| chunk.path.as_ref().unwrap().display().to_string())
        .collect::<Vec<_>>();

    assert_eq!(chunks.len(), 2);
    assert!(paths.contains(&"a.txt".to_owned()));
    assert!(paths.contains(&"b.txt".to_owned()));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn run_rg_honors_starts_with_without_scanning_other_roots() {
    let dir = temp_workspace();
    fs::create_dir_all(dir.join("src")).expect("create src");
    fs::create_dir_all(dir.join("docs")).expect("create docs");
    fs::write(dir.join("src/a.txt"), "hello needle\n").expect("write src/a.txt");
    fs::write(dir.join("docs/b.txt"), "needle in docs\n").expect("write docs/b.txt");

    let chunks = run_rg(
        SearchQuery::new("needle", dir.clone(), 10).with_path_filters(["src/"], [] as [&str; 0]),
    )
    .expect("rg search");
    let paths = chunks
        .iter()
        .map(|chunk| chunk.path.as_ref().unwrap().display().to_string())
        .collect::<Vec<_>>();

    assert_eq!(paths, ["src/a.txt"]);

    fs::write(dir.join("src/another.txt"), "needle\n").unwrap();
    for prefix in ["src/a", "./src/a"] {
        let chunks = run_rg(
            SearchQuery::new("needle", dir.clone(), 2).with_path_filters([prefix], [] as [&str; 0]),
        )
        .unwrap();
        assert_eq!(chunks.len(), 2, "{prefix}");
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.path.as_ref().unwrap().starts_with("src"))
        );
    }
    assert_eq!(
        run_rg(
            SearchQuery::new("needle", dir.clone(), 10)
                .with_path_filters(["src/", "src/a"], [] as [&str; 0])
        )
        .unwrap()
        .len(),
        2
    );
    // Existing directory name without '/' is still a prefix of siblings.
    fs::create_dir_all(dir.join("src-other")).unwrap();
    fs::write(dir.join("src-other/a.txt"), "needle\n").unwrap();
    assert_eq!(
        run_rg(
            SearchQuery::new("needle", dir.clone(), 10).with_path_filters(["src"], [] as [&str; 0])
        )
        .unwrap()
        .len(),
        3
    );
    assert!(
        run_rg(
            SearchQuery::new("needle", dir.clone(), 1)
                .with_path_filters(["missing/"], [] as [&str; 0])
        )
        .unwrap()
        .is_empty()
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn run_rg_preserves_single_file_and_colon_paths() {
    let dir = temp_workspace();
    fs::write(dir.join("part:one.txt"), "needle here\n").unwrap();
    let chunks = run_rg(
        SearchQuery::new("needle", dir.clone(), 10)
            .with_path_filters(["part:one.txt"], [] as [&str; 0]),
    )
    .unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].path.as_ref().unwrap(), Path::new("part:one.txt"));
    assert_eq!(chunks[0].metadata["line"], 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn run_rg_reports_invalid_pattern_but_not_no_matches() {
    let dir = temp_workspace();
    fs::write(dir.join("a.txt"), "hello\n").unwrap();
    assert!(
        run_rg(SearchQuery::new("absent", dir.clone(), 10))
            .unwrap()
            .is_empty()
    );
    let error = run_rg(SearchQuery::new("[", dir.clone(), 10)).unwrap_err();
    assert!(error.contains("regex parse error"), "{error}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn run_rg_keeps_bounded_location_for_long_matching_line() {
    let dir = temp_workspace();
    fs::write(
        dir.join("long.txt"),
        format!("needle{}\n", "x".repeat(4000)),
    )
    .unwrap();
    let chunks = run_rg(SearchQuery::new("needle", dir.clone(), 10)).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].path.as_ref().unwrap(), Path::new("long.txt"));
    assert_eq!(chunks[0].metadata["line"], 1);
    assert_eq!(chunks[0].content, "[Omitted long matching line]");
    fs::remove_dir_all(dir).unwrap();
}

fn temp_workspace() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "agent-rg-search-test-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp workspace");
    dir
}
