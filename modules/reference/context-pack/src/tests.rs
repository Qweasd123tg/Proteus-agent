use super::*;

#[test]
fn byte_budget_prefers_higher_score_and_restores_original_order() {
    let chunks = vec![
        ContextChunk::new("low", "11111").with_score(0.1),
        ContextChunk::new("high_a", "22222").with_score(0.9),
        ContextChunk::new("high_b", "33333").with_score(0.8),
    ];

    let selected = apply_byte_budget(chunks, 10);

    assert_eq!(
        selected
            .iter()
            .map(|chunk| chunk.source.as_str())
            .collect::<Vec<_>>(),
        vec!["high_a", "high_b"]
    );
}

#[test]
fn byte_budget_keeps_tie_score_order() {
    let chunks = vec![
        ContextChunk::new("first", "11111").with_score(0.5),
        ContextChunk::new("second", "22222").with_score(0.5),
        ContextChunk::new("third", "33333").with_score(0.5),
    ];

    let selected = apply_byte_budget(chunks, 10);

    assert_eq!(
        selected
            .iter()
            .map(|chunk| chunk.source.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
}

#[test]
fn bounded_workspace_read_reads_only_limit() {
    let dir = tempfile::tempdir().expect("workspace");
    let path = dir.path().join("large.txt");
    std::fs::write(&path, "abcdef").expect("large file");

    let content = read_bounded_workspace_utf8_file(dir.path(), &path, 3)
        .expect("bounded read")
        .expect("content");

    assert_eq!(content, "abc");
}

#[cfg(unix)]
#[test]
fn bounded_workspace_read_rejects_symlink_escape() {
    let dir = tempfile::tempdir().expect("workspace");
    let outside = tempfile::tempdir().expect("outside");
    let outside_file = outside.path().join("secret.txt");
    std::fs::write(&outside_file, "secret").expect("outside file");
    let link = dir.path().join("AGENTS.md");
    std::os::unix::fs::symlink(&outside_file, &link).expect("symlink");

    let content = read_bounded_workspace_utf8_file(dir.path(), &link, 100).expect("bounded read");

    assert!(content.is_none());
}

#[test]
fn project_instruction_chunks_layer_root_to_cwd_and_use_override_first() {
    let dir = tempfile::tempdir().expect("workspace");
    let root = dir.path();
    let service = root.join("services");
    let cwd = service.join("payments");
    std::fs::create_dir_all(&cwd).expect("nested cwd");
    std::fs::write(root.join("AGENTS.md"), "root rules\n").expect("root agents");
    std::fs::write(service.join("AGENTS.md"), "service rules\n").expect("service agents");
    std::fs::write(cwd.join("AGENTS.md"), "base payment rules\n").expect("cwd agents");
    std::fs::write(cwd.join("AGENTS.override.md"), "override payment rules\n")
        .expect("cwd override");
    let config = RepoAwareContextConfig::default();

    let chunks = project_instruction_chunks_from_root(&cwd, root, &config).expect("chunks");

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.path.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some(Path::new("AGENTS.md")),
            Some(Path::new("services/AGENTS.md")),
            Some(Path::new("services/payments/AGENTS.override.md")),
        ]
    );
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "root rules\n",
            "service rules\n",
            "override payment rules\n",
        ]
    );
}

#[test]
fn project_instruction_chunks_skip_empty_override_for_fallback_file() {
    let dir = tempfile::tempdir().expect("workspace");
    std::fs::write(dir.path().join("AGENTS.override.md"), "").expect("empty override");
    std::fs::write(dir.path().join("AGENTS.md"), "fallback rules\n").expect("agents");
    let config = RepoAwareContextConfig::default();

    let chunks =
        project_instruction_chunks_from_root(dir.path(), dir.path(), &config).expect("chunks");

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].path.as_deref(), Some(Path::new("AGENTS.md")));
    assert_eq!(chunks[0].content, "fallback rules\n");
}

#[test]
fn project_instruction_root_uses_git_root_when_available() {
    let dir = tempfile::tempdir().expect("workspace");
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(dir.path())
        .status();
    let Ok(status) = status else {
        return;
    };
    if !status.success() {
        return;
    }
    let cwd = dir.path().join("services/payments");
    std::fs::create_dir_all(&cwd).expect("nested cwd");

    let root = project_instruction_root(&cwd).expect("instruction root");

    assert_eq!(root, dir.path().canonicalize().expect("canonical root"));
}

#[test]
fn environment_chunks_report_current_platform_and_sh() {
    let input = ContextBuilderModuleInput {
        task: proteus_contracts::domain::AgentTask::new("task", PathBuf::from("/ws")),
        config: Value::Null,
    };

    let chunks = environment_chunks(&input);

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].source, "repo_aware:environment");
    let content = &chunks[0].content;
    assert!(content.starts_with(ENVIRONMENT_CONTEXT_TAG), "{content}");
    assert!(content.contains("<cwd>/ws</cwd>"), "{content}");
    assert!(
        content.contains(&format!(
            "<operating_system>{}</operating_system>",
            std::env::consts::OS
        )),
        "{content}"
    );
    assert!(
        content.contains(&format!(
            "<architecture>{}</architecture>",
            std::env::consts::ARCH
        )),
        "{content}"
    );
    assert!(
        content.contains(&format!("<shell>{EXEC_SHELL}</shell>")),
        "{content}"
    );
}

#[test]
fn extract_search_queries_keeps_domain_lowercase_terms() {
    let queries = extract_search_queries("почему approval не работает где shell policy?");

    assert_eq!(queries, vec!["approval", "shell", "policy"]);
}

#[test]
fn extract_search_queries_skips_common_lowercase_stopwords() {
    let queries = extract_search_queries("what should this context workflow inspect");

    assert_eq!(queries, vec!["context", "workflow", "inspect"]);
}

#[test]
fn extract_search_queries_dedupes_case_insensitively() {
    let queries = extract_search_queries("Workflow workflow ToolSafety tool_safety");

    assert_eq!(queries, vec!["Workflow", "ToolSafety", "tool_safety"]);
}
