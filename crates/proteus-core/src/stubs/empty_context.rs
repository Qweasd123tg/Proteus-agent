use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{ContextBuildInput, ContextBuilder},
    domain::ContextBundle,
};

#[derive(Debug, Default)]
pub struct EmptyContextBuilder;

#[async_trait]
impl ContextBuilder for EmptyContextBuilder {
    async fn build(&self, _input: ContextBuildInput) -> Result<ContextBundle> {
        Ok(ContextBundle::new(Vec::new()))
    }
}
