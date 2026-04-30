use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::{
    contracts::{ContextBuildInput, ContextBuilder, SearchQuery},
    domain::{ContextBundle, ContextChunk, MemoryQuery},
};

#[derive(Debug)]
pub struct SimpleContextBuilder {
    pub max_search_results: usize,
}

#[async_trait]
impl ContextBuilder for SimpleContextBuilder {
    async fn build(&self, input: ContextBuildInput) -> Result<ContextBundle> {
        let mut chunks = vec![
            ContextChunk::new("task", input.task.text.clone())
                .with_score(1.0)
                .with_metadata(json!({})),
        ];

        for item in input
            .memory
            .recall(MemoryQuery::new(input.task.text.clone(), 5))
            .await?
        {
            chunks.push(
                ContextChunk::new(format!("memory:{}", item.kind), item.content)
                    .with_metadata(item.metadata),
            );
        }

        chunks.extend(
            input
                .search
                .search(SearchQuery::new(
                    input.task.text.clone(),
                    input.task.cwd.clone(),
                    self.max_search_results,
                ))
                .await?,
        );

        let token_estimate = chunks
            .iter()
            .map(|chunk| chunk.content.len() / 4 + 1)
            .sum::<usize>() as u32;
        Ok(ContextBundle::new(chunks).with_token_estimate(token_estimate))
    }
}
