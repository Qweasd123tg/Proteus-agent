use super::*;

pub(super) fn build_codex_context(
    input: ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    config: CodexContextConfig,
) -> anyhow::Result<ContextBundle> {
    let repo_config = RepoAwareContextConfig::from(&config);
    let mut chunks = Vec::new();

    for provider in &config.providers {
        let mut provider_chunks = match provider.as_str() {
            "project_instructions" => codex_project_instruction_chunks(&input, &config)?,
            "environment" => environment_chunks(&input),
            "manifest" => manifest_chunks(&input, &repo_config)?,
            "git_status" => git_status_chunks(&input)?,
            "git_diff" => git_diff_chunks(&input, &config)?,
            "repo_tree" => repo_tree_chunks(&input, &repo_config)?,
            "memory" => memory_chunks(&input, host, &repo_config)?,
            "search" => search_chunks(&input, host, &repo_config)?,
            external => external_provider_chunks(&input, host, external)?,
        };
        if provider == "project_instructions" {
            let root = project_instruction_root(&input.task.cwd)?;
            for chunk in &mut provider_chunks {
                shape_codex_project_instructions(chunk, &root);
            }
        } else if provider == "environment" {
            for chunk in &mut provider_chunks {
                chunk.render_mode = ContextRenderMode::Verbatim;
            }
        }
        chunks.extend(retag_context_chunks(
            provider_chunks,
            "repo_aware",
            "codex_context",
        ));
    }

    let chunks = apply_byte_budget(chunks, config.max_context_bytes);
    let token_estimate = token_estimate(&chunks);
    Ok(ContextBundle::new(chunks)
        .with_summary(format!(
            "codex_context with {} providers",
            config.providers.len()
        ))
        .with_token_estimate(token_estimate))
}

fn codex_project_instruction_chunks(
    input: &ContextBuilderModuleInput,
    config: &CodexContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    let root = project_instruction_root(&input.task.cwd)?;
    codex_project_instruction_chunks_from_root(&input.task.cwd, &root, config)
}

fn codex_project_instruction_chunks_from_root(
    cwd: &Path,
    root: &Path,
    config: &CodexContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    let root = root.canonicalize()?;
    let mut remaining = config.project_doc_max_bytes;
    let mut chunks = Vec::new();

    for dir in project_instruction_dirs(&root, cwd)? {
        if remaining == 0 {
            break;
        }
        if let Some(chunk) = codex_project_instruction_chunk_for_dir(
            &root,
            &dir,
            &config.project_instruction_files,
            &mut remaining,
        )? {
            chunks.push(chunk);
        }
    }
    Ok(chunks)
}

fn codex_project_instruction_chunk_for_dir(
    root: &Path,
    dir: &Path,
    project_instruction_files: &[String],
    remaining: &mut usize,
) -> anyhow::Result<Option<ContextChunk>> {
    for file in project_instruction_files {
        let Some(relative_path) = safe_relative_path(file) else {
            continue;
        };
        let path = dir.join(&relative_path);
        let Some(content) =
            workspace_files::read_bounded_workspace_utf8_prefix(root, &path, *remaining)?
        else {
            continue;
        };
        if content.content.trim().is_empty() {
            continue;
        }

        *remaining = remaining.saturating_sub(content.bytes_read);
        let display_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        return Ok(Some(chunk(
            "repo_aware:project_instructions",
            Some(display_path),
            content.content,
            0.95,
            "project_instructions",
            "project instruction file",
        )));
    }
    Ok(None)
}

fn shape_codex_project_instructions(chunk: &mut ContextChunk, root: &Path) {
    let directory = chunk
        .path
        .as_deref()
        .and_then(Path::parent)
        .map(|parent| root.join(parent))
        .unwrap_or_else(|| root.to_path_buf());
    chunk.content = format!(
        "# AGENTS.md instructions for {}\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>",
        directory.display(),
        chunk.content.trim_end()
    );
    chunk.render_mode = ContextRenderMode::Verbatim;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_context_shapes_project_instructions_verbatim() {
        let root = Path::new("/workspace");
        let mut instructions =
            ContextChunk::new("repo_aware:project_instructions", "use cargo fmt\n")
                .with_path(PathBuf::from("services/AGENTS.md"))
                .with_metadata(metadata("project_instructions", "test", Value::Null));
        shape_codex_project_instructions(&mut instructions, root);

        assert_eq!(
            instructions.content,
            "# AGENTS.md instructions for /workspace/services\n\n<INSTRUCTIONS>\nuse cargo fmt\n</INSTRUCTIONS>"
        );
        assert_eq!(instructions.render_mode, ContextRenderMode::Verbatim);
    }

    #[test]
    fn codex_project_instruction_default_budget_is_32_kib() {
        assert_eq!(
            CodexContextConfig::default().project_doc_max_bytes,
            32 * 1024
        );
    }

    #[test]
    fn codex_project_instruction_loader_keeps_a_20_kib_agents_file_intact() {
        let dir = tempfile::tempdir().expect("workspace");
        let agents = "a".repeat(20 * 1024);
        std::fs::write(dir.path().join("AGENTS.md"), &agents).expect("agents");

        let chunks = codex_project_instruction_chunks_from_root(
            dir.path(),
            dir.path(),
            &CodexContextConfig::default(),
        )
        .expect("chunks");

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, agents);
    }

    #[test]
    fn codex_project_instruction_loader_uses_the_first_nonempty_override() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("AGENTS.override.md"), "override rules").expect("override");
        std::fs::write(dir.path().join("AGENTS.md"), "base rules").expect("agents");

        let chunks = codex_project_instruction_chunks_from_root(
            dir.path(),
            dir.path(),
            &CodexContextConfig::default(),
        )
        .expect("chunks");

        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].path.as_deref(),
            Some(Path::new("AGENTS.override.md"))
        );
        assert_eq!(chunks[0].content, "override rules");
    }

    #[test]
    fn codex_project_instruction_loader_shares_budget_from_root_to_cwd() {
        let dir = tempfile::tempdir().expect("workspace");
        let cwd = dir.path().join("nested");
        std::fs::create_dir(&cwd).expect("nested cwd");
        std::fs::write(dir.path().join("AGENTS.md"), "root").expect("root agents");
        std::fs::write(cwd.join("AGENTS.md"), "abcdef").expect("child agents");
        let config = CodexContextConfig {
            project_doc_max_bytes: 7,
            ..CodexContextConfig::default()
        };

        let chunks =
            codex_project_instruction_chunks_from_root(&cwd, dir.path(), &config).expect("chunks");

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.content.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "abc"]
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.path.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some(Path::new("AGENTS.md")),
                Some(Path::new("nested/AGENTS.md"))
            ]
        );
    }

    #[test]
    fn codex_project_instruction_loader_honors_profile_budget_override() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("AGENTS.md"), "abcdefgh").expect("agents");
        let config = config_or_default::<CodexContextConfig>(json!({
            "project_doc_max_bytes": 3,
            "project_instruction_files": ["AGENTS.md"],
        }))
        .expect("profile config");

        let chunks = codex_project_instruction_chunks_from_root(dir.path(), dir.path(), &config)
            .expect("chunks");

        assert_eq!(chunks[0].content, "abc");
    }

    #[test]
    fn codex_project_instruction_loader_truncates_at_a_utf8_boundary() {
        let dir = tempfile::tempdir().expect("workspace");
        let cwd = dir.path().join("nested");
        std::fs::create_dir(&cwd).expect("nested cwd");
        std::fs::write(dir.path().join("AGENTS.md"), "Привет").expect("agents");
        std::fs::write(cwd.join("AGENTS.md"), "z").expect("child agents");
        let config = CodexContextConfig {
            project_doc_max_bytes: 7,
            ..CodexContextConfig::default()
        };

        let chunks =
            codex_project_instruction_chunks_from_root(&cwd, dir.path(), &config).expect("chunks");

        assert_eq!(chunks[0].content, "При");
        assert_eq!(
            chunks.len(),
            1,
            "partial UTF-8 byte consumes the raw budget"
        );
        assert!(chunks[0].content.len() <= config.project_doc_max_bytes);
    }
}
