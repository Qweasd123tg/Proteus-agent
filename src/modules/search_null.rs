use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{SearchBackend, SearchQuery},
    domain::ContextChunk,
};

#[derive(Debug)]
pub struct NullSearch;

#[async_trait]
impl SearchBackend for NullSearch {
    async fn search(&self, _query: SearchQuery) -> Result<Vec<ContextChunk>> {
        Ok(Vec::new())
    }
}
