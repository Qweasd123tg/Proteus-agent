use super::*;
use crate::search_queries::extract_search_queries;

pub(super) fn memory_chunks(
    input: &ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    config: &RepoAwareContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    let mut chunks = recall_memory(
        input,
        host,
        config.memory_provider.as_deref(),
        MemoryQuery::new(input.task.text.clone(), config.memory_limit),
    )?;
    for chunk in &mut chunks {
        chunk.source = format!("repo_aware:{}", chunk.source);
        chunk.score = Some(0.7);
        chunk.metadata = metadata("memory", "memory recall", chunk.metadata.clone());
    }
    Ok(chunks)
}

pub(super) fn search_chunks(
    input: &ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    config: &RepoAwareContextConfig,
) -> anyhow::Result<Vec<ContextChunk>> {
    if config.max_search_results == 0 || config.search_provider.is_none() {
        return Ok(Vec::new());
    }
    let queries = extract_search_queries(&input.task.text);
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    let per_query_limit = config.max_search_results.div_ceil(queries.len()).max(1);
    let mut chunks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for query in queries {
        let results = search_best_effort(
            input,
            host,
            config.search_provider.as_deref(),
            SearchQuery::new(query.clone(), input.task.cwd.clone(), per_query_limit)
                .with_use_case("repo_aware_context"),
            "repo_aware_context",
        )?;
        for mut chunk in results {
            let dedupe_key = format!(
                "{}\n{}\n{}",
                chunk
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                chunk.content,
                chunk.source
            );
            if !seen.insert(dedupe_key) {
                continue;
            }
            chunk.source = format!("repo_aware:search:{}", chunk.source);
            chunk.score = chunk.score.or(Some(0.55));
            chunk.metadata = metadata(
                "search",
                "search result",
                metadata_with(chunk.metadata.clone(), "query", json!(query)),
            );
            chunks.push(chunk);
            if chunks.len() >= config.max_search_results {
                return Ok(chunks);
            }
        }
    }
    Ok(chunks)
}

pub(super) fn external_provider_chunks(
    input: &ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    provider_id: &str,
) -> anyhow::Result<Vec<ContextChunk>> {
    provider_chunks(input, host, provider_id, Value::Null)
}

fn provider_chunks(
    input: &ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    provider_id: &str,
    query: Value,
) -> anyhow::Result<Vec<ContextChunk>> {
    let request = ProcessContextProviderInput {
        provider_id: provider_id.to_owned(),
        task: input.task.clone(),
        metadata: query,
    };
    let output = host
        .context_provider_json(provider_id.to_owned(), serde_json::to_string(&request)?)
        .map_err(|error| anyhow::anyhow!("{}", error.message))?;
    Ok(serde_json::from_str(&output)?)
}

pub(super) fn search_best_effort(
    input: &ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    provider_id: Option<&str>,
    query: SearchQuery,
    provider: &str,
) -> anyhow::Result<Vec<ContextChunk>> {
    let Some(provider_id) = provider_id else {
        return Ok(Vec::new());
    };
    match provider_chunks(input, host, provider_id, serde_json::to_value(query)?) {
        Ok(chunks) => Ok(chunks),
        Err(error) => Ok(vec![
            ContextChunk::new(
                format!("{provider}:search_error"),
                format!("Workspace search was skipped: {error}"),
            )
            .with_score(0.05)
            .with_metadata(metadata(
                "search",
                "search backend error; turn should continue without search context",
                json!({"error": error.to_string()}),
            )),
        ]),
    }
}

pub(super) fn recall_memory(
    input: &ContextBuilderModuleInput,
    host: &mut ContextBuilderModuleHostMut<'_>,
    provider_id: Option<&str>,
    query: MemoryQuery,
) -> anyhow::Result<Vec<ContextChunk>> {
    let Some(provider_id) = provider_id else {
        return Ok(Vec::new());
    };
    provider_chunks(input, host, provider_id, serde_json::to_value(query)?)
}
