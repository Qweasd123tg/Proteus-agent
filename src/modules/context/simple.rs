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
        let mut chunks = vec![ContextChunk {
            source: "task".to_owned(),
            path: None,
            content: input.task.text.clone(),
            score: Some(1.0),
            metadata: json!({}),
        }];

        for item in input
            .memory
            .recall(MemoryQuery {
                text: input.task.text.clone(),
                limit: 5,
            })
            .await?
        {
            chunks.push(ContextChunk {
                source: format!("memory:{}", item.kind),
                path: None,
                content: item.content,
                score: None,
                metadata: item.metadata,
            });
        }

        chunks.extend(
            input
                .search
                .search(SearchQuery {
                    text: input.task.text.clone(),
                    cwd: input.task.cwd.clone(),
                    max_results: self.max_search_results,
                })
                .await?,
        );

        let token_estimate = chunks
            .iter()
            .map(|chunk| chunk.content.len() / 4 + 1)
            .sum::<usize>() as u32;
        Ok(ContextBundle {
            chunks,
            summary: None,
            token_estimate: Some(token_estimate),
        })
    }
}
