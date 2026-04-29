use anyhow::{Result, bail};
use serde_json::Value;

use crate::{
    contracts::{RenderComponent, Renderer},
    core::{ContextIndicatorComponentConfig, ModelNameComponentConfig, StatuslineRendererConfig},
    domain::AgentOutput,
};

pub struct StatuslineRenderer {
    components: Vec<Box<dyn RenderComponent>>,
    position: String,
    frame: String,
    separator: String,
    ansi: bool,
}

impl StatuslineRenderer {
    pub fn from_config(config: &StatuslineRendererConfig) -> Result<Self> {
        match config.position.as_str() {
            "top" | "bottom" => {}
            position => bail!("unsupported statusline position: {position}"),
        }
        match config.frame.as_str() {
            "line" | "block" => {}
            frame => bail!("unsupported statusline frame: {frame}"),
        }

        let components = build_components(config)?;
        Ok(Self {
            components,
            position: config.position.clone(),
            frame: config.frame.clone(),
            separator: config.separator.clone(),
            ansi: config.ansi,
        })
    }
}

#[async_trait::async_trait]
impl Renderer for StatuslineRenderer {
    async fn render(&self, output: &AgentOutput) -> Result<String> {
        let parts = self
            .components
            .iter()
            .filter_map(|component| component.render(output).transpose())
            .collect::<Result<Vec<_>>>()?;

        if parts.is_empty() {
            return Ok(output.text.clone());
        }

        let statusline = parts.join(&self.separator);
        let mut status = match self.frame.as_str() {
            "line" => statusline,
            "block" => status_block(&statusline),
            frame => bail!("unsupported statusline frame: {frame}"),
        };
        if self.ansi {
            status = format!("\x1b[2m{status}\x1b[0m");
        }

        match self.position.as_str() {
            "bottom" if output.text.ends_with('\n') => Ok(format!("{}{}", output.text, status)),
            "bottom" => Ok(format!("{}\n{}", output.text, status)),
            "top" => Ok(format!("{}\n{}", status, output.text)),
            position => bail!("unsupported statusline position: {position}"),
        }
    }
}

fn build_components(config: &StatuslineRendererConfig) -> Result<Vec<Box<dyn RenderComponent>>> {
    config
        .components
        .iter()
        .map(|component| match component.as_str() {
            "model" => {
                Ok(Box::new(ModelNameComponent::new(config.model.clone()))
                    as Box<dyn RenderComponent>)
            }
            "context" => Ok(
                Box::new(ContextIndicatorComponent::new(config.context.clone()))
                    as Box<dyn RenderComponent>,
            ),
            "session" => Ok(Box::new(SessionComponent) as Box<dyn RenderComponent>),
            name => bail!("unsupported statusline component: {name}"),
        })
        .collect()
}

struct ModelNameComponent {
    config: ModelNameComponentConfig,
}

impl ModelNameComponent {
    fn new(config: ModelNameComponentConfig) -> Self {
        Self { config }
    }
}

impl RenderComponent for ModelNameComponent {
    fn key(&self) -> &'static str {
        "model"
    }

    fn render(&self, output: &AgentOutput) -> Result<Option<String>> {
        let Some(model) = output.metadata.get("model") else {
            return Ok(None);
        };
        let model_name = model
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| model.get("model").and_then(Value::as_str));
        let provider = model.get("provider").and_then(Value::as_str);

        let Some(model_name) = model_name else {
            return Ok(None);
        };

        let value = if self.config.show_provider {
            match provider {
                Some(provider) if !provider.is_empty() => {
                    format!("{provider}/{model_name}")
                }
                _ => model_name.to_owned(),
            }
        } else {
            model_name.to_owned()
        };

        Ok(Some(format!("{} {}", self.config.label, value)))
    }
}

struct ContextIndicatorComponent {
    config: ContextIndicatorComponentConfig,
}

impl ContextIndicatorComponent {
    fn new(config: ContextIndicatorComponentConfig) -> Self {
        Self { config }
    }
}

impl RenderComponent for ContextIndicatorComponent {
    fn key(&self) -> &'static str {
        "context"
    }

    fn render(&self, output: &AgentOutput) -> Result<Option<String>> {
        let Some(context) = output.metadata.get("context") else {
            return Ok(None);
        };
        let token_estimate = context
            .get("token_estimate")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let chunks = context
            .get("chunks")
            .and_then(Value::as_u64)
            .unwrap_or_default();

        let chunk_word = if chunks == 1 { "chunk" } else { "chunks" };
        let Some(max_tokens) = self.config.max_tokens.filter(|max| *max > 0) else {
            return Ok(Some(format!(
                "{} {}t {} {}",
                self.config.label, token_estimate, chunks, chunk_word
            )));
        };

        let percent = ((token_estimate as f64 / max_tokens as f64) * 100.0).clamp(0.0, 100.0);
        let bar = context_bar(percent, self.config.bar_width);
        Ok(Some(format!(
            "{} [{}] {:.0}% {}t/{}t {} {}",
            self.config.label, bar, percent, token_estimate, max_tokens, chunks, chunk_word
        )))
    }
}

struct SessionComponent;

impl RenderComponent for SessionComponent {
    fn key(&self) -> &'static str {
        "session"
    }

    fn render(&self, output: &AgentOutput) -> Result<Option<String>> {
        let Some(session_id) = output.metadata.get("session_id").and_then(Value::as_str) else {
            return Ok(None);
        };
        let short_id = session_id.get(..8).unwrap_or(session_id);
        Ok(Some(format!("session {short_id}")))
    }
}

fn context_bar(percent: f64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = ((percent / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "#".repeat(filled), "-".repeat(width - filled))
}

fn status_block(statusline: &str) -> String {
    const TITLE: &str = "актуально";
    let text_width = statusline.chars().count().max(72);
    let inner_width = text_width + 2;
    let title = format!(" {TITLE} ");
    let right = inner_width.saturating_sub(title.chars().count());
    format!(
        "╭{}{}╮\n│ {}{} │\n╰{}╯",
        title,
        "─".repeat(right),
        statusline,
        " ".repeat(text_width.saturating_sub(statusline.chars().count())),
        "─".repeat(inner_width)
    )
}
