use anyhow::Result;
use async_trait::async_trait;

use crate::{contracts::Renderer, domain::AgentOutput};

#[derive(Debug)]
pub struct PlainRenderer;

#[async_trait]
impl Renderer for PlainRenderer {
    async fn render(&self, output: &AgentOutput) -> Result<String> {
        Ok(output.text.clone())
    }
}
