use anyhow::Result;
use async_trait::async_trait;

use crate::domain::AgentOutput;

#[async_trait]
pub trait Renderer: Send + Sync {
    async fn render(&self, output: &AgentOutput) -> Result<String>;
}
