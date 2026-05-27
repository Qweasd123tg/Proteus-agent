use proteus_contracts::abi_stable::std_types::{RResult, RString};

use crate::contracts::{RenderError, Renderer, parse_output_json};

#[derive(Debug, Default)]
pub struct TextRenderer;

impl Renderer for TextRenderer {
    fn render_json(&self, output_json: RString) -> RResult<RString, RenderError> {
        match parse_output_json(output_json.as_str()) {
            Ok(output) => RResult::ROk(output.text.into()),
            Err(error) => RResult::RErr(RenderError::new(format!(
                "failed to parse agent output: {error}"
            ))),
        }
    }
}
