use anyhow::Result;

use crate::domain::AgentOutput;

pub trait RenderComponent: Send + Sync {
    fn key(&self) -> &'static str;

    fn render(&self, output: &AgentOutput) -> Result<Option<String>>;
}
