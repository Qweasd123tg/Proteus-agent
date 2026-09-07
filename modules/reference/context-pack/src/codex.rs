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
            "project_instructions" => project_instruction_chunks(&input, &repo_config)?,
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
}
