use anyhow::Result;

use crate::{contracts::Renderer, domain::AgentOutput};

#[derive(Debug, Default)]
pub struct TextRenderer;

impl Renderer for TextRenderer {
    fn render(&self, output: &AgentOutput) -> Result<String> {
        Ok(output.text.clone())
    }
}
