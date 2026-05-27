//! Renderer plugin pack.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use anyhow::{Result, bail};
#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::abi_stable::{export_root_module, prefix_type::PrefixTypeTrait};
use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    contracts::{RenderError, Renderer, parse_output_json},
    domain::AgentOutput,
};
#[cfg(feature = "plugin-entrypoint")]
use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RStr, RString as AbiRString},
    },
    contracts::{Renderer_TO, RendererObject},
    plugin::{PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref},
};
use serde_json::Value;

#[derive(Default)]
pub struct PlainRendererPlugin;

impl Renderer for PlainRendererPlugin {
    fn render_json(&self, output_json: RString) -> RResult<RString, RenderError> {
        let output = match parse_output_json(output_json.as_str()) {
            Ok(output) => output,
            Err(error) => {
                return RResult::RErr(RenderError::new(format!(
                    "failed to parse agent output: {error}"
                )));
            }
        };
        RResult::ROk(output.text.into())
    }
}

#[derive(Default)]
pub struct StatuslineRendererPlugin {
    config: StatuslineConfig,
}

impl StatuslineRendererPlugin {
    pub fn with_config(config: StatuslineConfig) -> Self {
        Self { config }
    }
}

impl Renderer for StatuslineRendererPlugin {
    fn render_json(&self, output_json: RString) -> RResult<RString, RenderError> {
        let output = match parse_output_json(output_json.as_str()) {
            Ok(output) => output,
            Err(error) => {
                return RResult::RErr(RenderError::new(format!(
                    "failed to parse agent output: {error}"
                )));
            }
        };
        match render_statusline(&self.config, &output) {
            Ok(text) => RResult::ROk(text.into()),
            Err(error) => RResult::RErr(RenderError::new(error.to_string())),
        }
    }
}

pub struct StatuslineConfig {
    pub components: Vec<String>,
    pub position: String,
    pub frame: String,
    pub separator: String,
    pub ansi: bool,
    pub model: ModelNameConfig,
    pub context: ContextIndicatorConfig,
}

impl Default for StatuslineConfig {
    fn default() -> Self {
        Self {
            components: ["model", "context", "session"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            position: "bottom".to_owned(),
            frame: "block".to_owned(),
            separator: " | ".to_owned(),
            ansi: true,
            model: ModelNameConfig::default(),
            context: ContextIndicatorConfig::default(),
        }
    }
}

pub struct ModelNameConfig {
    pub label: String,
    pub show_provider: bool,
}

impl Default for ModelNameConfig {
    fn default() -> Self {
        Self {
            label: "model".to_owned(),
            show_provider: true,
        }
    }
}

pub struct ContextIndicatorConfig {
    pub label: String,
    pub max_tokens: Option<u32>,
    pub bar_width: usize,
}

impl Default for ContextIndicatorConfig {
    fn default() -> Self {
        Self {
            label: "ctx".to_owned(),
            max_tokens: Some(200_000),
            bar_width: 10,
        }
    }
}

fn render_statusline(config: &StatuslineConfig, output: &AgentOutput) -> Result<String> {
    match config.position.as_str() {
        "top" | "bottom" => {}
        position => bail!("unsupported statusline position: {position}"),
    }
    match config.frame.as_str() {
        "line" | "block" => {}
        frame => bail!("unsupported statusline frame: {frame}"),
    }

    let parts = config
        .components
        .iter()
        .filter_map(|component| render_component(config, component, output).transpose())
        .collect::<Result<Vec<_>>>()?;

    if parts.is_empty() {
        return Ok(output.text.clone());
    }

    let statusline = parts.join(&config.separator);
    let mut status = match config.frame.as_str() {
        "line" => statusline,
        "block" => status_block(&statusline),
        frame => bail!("unsupported statusline frame: {frame}"),
    };
    if config.ansi {
        status = format!("\x1b[2m{status}\x1b[0m");
    }

    match config.position.as_str() {
        "bottom" if output.text.ends_with('\n') => Ok(format!("{}{}", output.text, status)),
        "bottom" => Ok(format!("{}\n{}", output.text, status)),
        "top" => Ok(format!("{}\n{}", status, output.text)),
        position => bail!("unsupported statusline position: {position}"),
    }
}

fn render_component(
    config: &StatuslineConfig,
    component: &str,
    output: &AgentOutput,
) -> Result<Option<String>> {
    match component {
        "model" => render_model(&config.model, output),
        "context" => render_context(&config.context, output),
        "session" => Ok(render_session(output)),
        name => bail!("unsupported statusline component: {name}"),
    }
}

fn render_model(config: &ModelNameConfig, output: &AgentOutput) -> Result<Option<String>> {
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

    let value = if config.show_provider {
        match provider {
            Some(provider) if !provider.is_empty() => format!("{provider}/{model_name}"),
            _ => model_name.to_owned(),
        }
    } else {
        model_name.to_owned()
    };

    Ok(Some(format!("{} {}", config.label, value)))
}

fn render_context(config: &ContextIndicatorConfig, output: &AgentOutput) -> Result<Option<String>> {
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
    let Some(max_tokens) = config.max_tokens.filter(|max| *max > 0) else {
        return Ok(Some(format!(
            "{} {}t {} {}",
            config.label, token_estimate, chunks, chunk_word
        )));
    };

    let percent = ((token_estimate as f64 / max_tokens as f64) * 100.0).clamp(0.0, 100.0);
    let bar = context_bar(percent, config.bar_width);
    Ok(Some(format!(
        "{} [{}] {:.0}% {}t/{}t {} {}",
        config.label, bar, percent, token_estimate, max_tokens, chunks, chunk_word
    )))
}

fn render_session(output: &AgentOutput) -> Option<String> {
    let session_id = output.metadata.get("session_id").and_then(Value::as_str)?;
    let short_id = session_id.get(..8).unwrap_or(session_id);
    Some(format!("session {short_id}"))
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

#[cfg(feature = "plugin-entrypoint")]
extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let plain: RendererObject = Renderer_TO::from_value(PlainRendererPlugin, TD_Opaque);
    if let RResult::RErr(error) = registry.register_renderer(AbiRString::from("plain"), plain) {
        return RResult::RErr(error);
    }

    let statusline: RendererObject =
        Renderer_TO::from_value(StatuslineRendererPlugin::default(), TD_Opaque);
    registry.register_renderer(AbiRString::from("statusline"), statusline)
}

#[cfg(feature = "plugin-entrypoint")]
#[export_root_module]
pub fn instantiate_root_module() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("renderer-pack"),
        description: RStr::from_str("Renderer plugins: plain and statusline"),
        register_modules,
    }
    .leak_into_prefix()
}
