use agent_contracts::abi_stable::std_types::{RResult, RString};

use crate::contracts::{RenderError, Renderer, parse_output_json};

#[derive(Debug)]
pub struct PlainRenderer;

impl Renderer for PlainRenderer {
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
