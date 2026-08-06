use super::*;

#[test]
fn plugin_context_provider_rejects_empty_and_duplicate_ids() {
    let mut catalog = ModuleCatalog::new();
    let empty_error = catalog
        .register_plugin_context_provider(" ", noop_plugin_context_provider())
        .unwrap_err();
    assert!(empty_error.to_string().contains("id must not be empty"));

    catalog
        .register_plugin_context_provider("hello", noop_plugin_context_provider())
        .unwrap();
    let duplicate_error = catalog
        .register_plugin_context_provider("hello", noop_plugin_context_provider())
        .unwrap_err();
    assert!(
        duplicate_error
            .to_string()
            .contains("context provider 'hello' is already registered")
    );
}

#[tokio::test]
async fn swapping_context_builder_does_not_change_runtime() {
    for context in ["simple", "repo_aware", "codex_context"] {
        let mut config = test_config();
        config.modules.context = context.to_owned();
        set_repo_aware_config(&mut config, json!({ "providers": ["repo_tree"] }));
        set_codex_context_config(&mut config, json!({ "providers": ["repo_tree"] }));

        let (output, events) = run_with(config, "summarize context").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn swapping_compactor_does_not_change_runtime() {
    for compactor in ["none", "codex"] {
        let mut config = test_config();
        config.modules.compactor = compactor.to_owned();

        let (output, events) = run_with(config, "summarize compacted context").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn unknown_repo_aware_provider_is_rejected_when_context_is_built() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    set_repo_aware_config(&mut config, json!({ "providers": ["mystery"] }));

    let registry = registry_from_test_config(&config, dir.path());
    let error = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new("summarize repo".to_owned(), dir.path().to_path_buf()),
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .expect_err("unknown repo_aware provider should be rejected");

    assert!(
        error
            .to_string()
            .contains("unknown context provider: mystery")
    );
}

#[tokio::test]
async fn codex_context_collects_codex_ordered_chunks_and_git_diff() {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }

    let dir = temp_workspace();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .expect("git init");
    std::fs::write(dir.path().join("AGENTS.md"), "Use focused tests.\n").expect("agents");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("manifest");
    std::fs::write(dir.path().join("tracked.txt"), "old\n").expect("tracked");
    std::process::Command::new("git")
        .args(["add", "tracked.txt"])
        .current_dir(dir.path())
        .output()
        .expect("git add");
    std::fs::write(dir.path().join("tracked.txt"), "new\n").expect("modified");
    std::fs::create_dir_all(dir.path().join("examples/source")).expect("examples/source");
    std::fs::write(dir.path().join("examples/source/skip.rs"), "skip me\n").expect("skip file");

    let mut config = test_config();
    config.modules.context = "codex_context".to_owned();
    set_codex_context_config(
        &mut config,
        json!({
            "providers": ["project_instructions", "git_status", "git_diff", "repo_tree", "manifest"],
            "max_context_bytes": 60000,
            "git_diff_max_bytes": 4000,
            "repo_tree_skip_entries": ["examples/source", ".git"],
        }),
    );
    let registry = registry_from_test_config(&config, dir.path());
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new("fix tracked file".to_owned(), dir.path().to_path_buf()),
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .unwrap();

    assert!(
        bundle
            .summary
            .as_deref()
            .unwrap_or("")
            .contains("codex_context")
    );
    assert!(
        !bundle
            .chunks
            .iter()
            .any(|chunk| chunk.source == "codex_context:task")
    );
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "codex_context:project_instructions"
            && chunk.metadata["context_profile"] == "codex_context"
            && chunk.content.contains("focused tests")
    }));
    assert!(
        bundle
            .chunks
            .iter()
            .any(|chunk| chunk.source == "codex_context:git_status"
                && chunk.content.contains("tracked.txt"))
    );
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "codex_context:git_diff"
            && chunk.content.contains("tracked.txt")
            && chunk.content.contains("-old")
            && chunk.content.contains("+new")
    }));
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "codex_context:manifest" && chunk.content.contains("name = \"demo\"")
    }));
    assert!(
        !bundle
            .chunks
            .iter()
            .any(|chunk| chunk.source == "codex_context:repo_tree"
                && chunk.content.contains("examples/source"))
    );
}

#[tokio::test]
async fn repo_aware_context_collects_provider_chunks_with_metadata() {
    let dir = temp_workspace();
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "Run cargo test before finishing.\n",
    )
    .expect("agents");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.path().join("src/core")).expect("src/core");
    std::fs::write(dir.path().join("src.rs"), "fn main() {}\n").expect("source");
    std::fs::write(
        dir.path().join("src/core/runtime.rs"),
        "pub struct Runtime;\n",
    )
    .expect("nested source");
    std::fs::create_dir_all(dir.path().join("target/debug")).expect("target dir");
    std::fs::write(dir.path().join("target/debug/build.log"), "skip me\n").expect("target file");
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    set_repo_aware_config(
        &mut config,
        json!({ "providers": ["project_instructions", "manifest", "repo_tree"] }),
    );
    let registry = registry_from_test_config(&config, dir.path());
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new("summarize repo".to_owned(), dir.path().to_path_buf()),
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .unwrap();

    assert!(
        bundle
            .chunks
            .iter()
            .any(|chunk| chunk.source == "repo_aware:task")
    );
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:project_instructions"
            && chunk.path.as_deref() == Some(std::path::Path::new("AGENTS.md"))
            && chunk.content.contains("cargo test")
            && chunk.metadata["provider"] == "project_instructions"
            && chunk.metadata["reason"] == "project instruction file"
    }));
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:manifest"
            && chunk.path.as_deref() == Some(std::path::Path::new("Cargo.toml"))
            && chunk.content.contains("name = \"demo\"")
    }));
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:repo_tree" && chunk.content.contains("src.rs")
    }));
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:repo_tree" && chunk.content.contains("src/core/runtime.rs")
    }));
    assert!(!bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:repo_tree" && chunk.content.contains("target/debug")
    }));
}

#[tokio::test]
async fn skills_provider_and_tool_compose_through_runtime_catalog() {
    let dir = temp_workspace();
    std::fs::create_dir(dir.path().join(".git")).expect("git marker");
    let skill_dir = dir.path().join(".proteus/skills/catalog-check");
    std::fs::create_dir_all(&skill_dir).expect("skill directory");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: catalog-check\ndescription: Check composed skill wiring\n---\n\nUse the composed runtime path.\n",
    )
    .expect("skill file");
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    config.tools.enabled = vec!["skill".to_owned()];
    set_repo_aware_config(&mut config, json!({ "providers": ["skills"] }));

    let registry = registry_from_test_config(&config, dir.path());
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new("use project skills".to_owned(), dir.path().to_path_buf()),
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .expect("skills context");
    let chunk = bundle
        .chunks
        .iter()
        .find(|chunk| chunk.source == "repo_aware:skills")
        .expect("available skills chunk");
    assert!(chunk.content.contains("<available_skills>"));
    assert!(chunk.content.contains("<name>catalog-check</name>"));
    assert!(!chunk.content.contains("Use the composed runtime path."));

    let tool = registry.tools.get("skill").expect("skill tool");
    let result = tool
        .invoke(
            &ToolCall::new(new_call_id(), "skill", json!({ "name": "catalog-check" })),
            ToolContext::new(dir.path().to_path_buf(), test_tool_owner()),
        )
        .await
        .expect("skill invocation");

    assert!(result.ok, "{result:?}");
    assert_eq!(result.output, "Use the composed runtime path.");
}

#[tokio::test]
async fn repo_aware_context_does_not_read_configured_paths_outside_workspace() {
    let dir = temp_workspace();
    let outside = tempfile::tempdir().expect("outside dir");
    std::fs::write(outside.path().join("secret.md"), "do not read").expect("secret");
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    set_repo_aware_config(
        &mut config,
        json!({
            "providers": ["project_instructions"],
            "project_instruction_files": [format!(
                "../{}/secret.md",
                outside.path().file_name().unwrap().to_string_lossy()
            )],
        }),
    );
    let registry = registry_from_test_config(&config, dir.path());
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new("summarize repo".to_owned(), dir.path().to_path_buf()),
            search: Arc::new(NullSearch),
            memory: Arc::new(NoMemory),
        })
        .await
        .unwrap();

    assert!(
        !bundle
            .chunks
            .iter()
            .any(|chunk| chunk.content.contains("do not read"))
    );
}

#[tokio::test]
async fn repo_aware_search_extracts_targeted_queries_from_task() {
    let dir = temp_workspace();
    let mut config = test_config();
    config.modules.context = "repo_aware".to_owned();
    set_repo_aware_config(
        &mut config,
        json!({
            "providers": ["search"],
            "max_search_results": 4,
        }),
    );
    let registry = registry_from_test_config(&config, dir.path());
    let queries = Arc::new(Mutex::new(Vec::new()));
    let bundle = registry
        .context
        .build(ContextBuildInput {
            task: AgentTask::new(
                "почему approval не работает где PermissionMode режет shell в ToolOrchestrator"
                    .to_owned(),
                dir.path().to_path_buf(),
            ),
            search: Arc::new(RecordingSearch {
                queries: queries.clone(),
            }),
            memory: Arc::new(NoMemory),
        })
        .await
        .unwrap();

    let queries = queries.lock().unwrap().clone();
    assert!(queries.iter().any(|query| query == "PermissionMode"));
    assert!(queries.iter().any(|query| query == "ToolOrchestrator"));
    assert!(
        !queries
            .iter()
            .any(|query| query.contains("почему approval"))
    );
    assert!(bundle.chunks.iter().any(|chunk| {
        chunk.source == "repo_aware:search:recording"
            && chunk.metadata["provider"] == "search"
            && chunk.metadata["query"] == "PermissionMode"
    }));
}

#[tokio::test]
async fn swapping_search_backend_does_not_change_runtime() {
    for search in ["null"] {
        let mut config = test_config();
        config.modules.search = search.to_owned();

        let (output, events) = run_with(config, "summarize hello").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}

#[tokio::test]
async fn swapping_memory_store_does_not_change_runtime() {
    for memory in ["none", "jsonl"] {
        let mut config = test_config();
        config.modules.memory = memory.to_owned();

        let (output, events) = run_with(config, "summarize memory").await;

        assert!(output.contains("Fake final answer"));
        assert!(events.events().await.len() >= 5);
    }
}
#[derive(Debug)]
struct RecordingSearch {
    queries: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SearchBackend for RecordingSearch {
    async fn search(&self, query: SearchQuery) -> anyhow::Result<Vec<ContextChunk>> {
        self.queries.lock().unwrap().push(query.text.clone());
        Ok(vec![
            ContextChunk::new("recording", format!("hit {}", query.text))
                .with_path("src/core/tool_orchestrator.rs".into()),
        ])
    }
}
