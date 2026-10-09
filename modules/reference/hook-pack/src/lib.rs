//! Owner-configured instructions and tool output budgeting contributions.
mod config_schema;
pub use config_schema::config_schema;
use proteus_contracts::{
    contracts::{HookEvent, HookInput, HookResponse},
    model_standard::{InstructionBlock, InstructionKind},
    process_module::{HookModule, ModuleRegistry, ProcessModuleError, ProcessModuleResult},
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstructionsConfig {
    text: String,
    placement: Placement,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Placement {
    Prepend,
    Append,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetConfig {
    max_bytes: usize,
    head_bytes: usize,
}
struct Instructions(InstructionsConfig);
struct OutputBudget(BudgetConfig);
fn error(error: impl std::fmt::Display) -> ProcessModuleError {
    ProcessModuleError::new(error.to_string())
}
impl HookModule for Instructions {
    fn invoke_json(&self, input_json: String) -> ProcessModuleResult<String> {
        let input: HookInput = serde_json::from_str(&input_json).map_err(error)?;
        let result = match input.event {
            HookEvent::BeforeModel { request, .. } => {
                let mut instructions = request.instructions;
                let block = InstructionBlock::new(InstructionKind::Developer, &self.0.text, 128);
                match self.0.placement {
                    Placement::Prepend => instructions.insert(0, block),
                    Placement::Append => instructions.push(block),
                }
                HookResponse::ModelContext {
                    messages: request.messages,
                    instructions,
                }
            }
            _ => HookResponse::Continue,
        };
        serde_json::to_string(&result).map_err(error)
    }
}
impl HookModule for OutputBudget {
    fn invoke_json(&self, input_json: String) -> ProcessModuleResult<String> {
        let input: HookInput = serde_json::from_str(&input_json).map_err(error)?;
        let result = match input.event {
            HookEvent::AfterTool { result, .. } if result.output.len() > self.0.max_bytes => {
                let text = result.output;
                let mut head = self.0.head_bytes;
                while !text.is_char_boundary(head) {
                    head -= 1;
                }
                let mut tail = text.len() - (self.0.max_bytes - self.0.head_bytes);
                while !text.is_char_boundary(tail) {
                    tail += 1;
                }
                HookResponse::ToolOutput {
                    output: format!("{}{}", &text[..head], &text[tail..]),
                }
            }
            _ => HookResponse::Continue,
        };
        serde_json::to_string(&result).map_err(error)
    }
}
pub fn register_hook(
    registry: &mut dyn ModuleRegistry,
    module_id: &str,
) -> ProcessModuleResult<()> {
    let hook: Box<dyn HookModule> = match module_id {
        "hook.instructions" => {
            let config: InstructionsConfig =
                serde_json::from_value(registry.module_config().clone()).map_err(error)?;
            if config.text.trim().is_empty() {
                return Err(error("hook.instructions text must not be blank"));
            }
            Box::new(Instructions(config))
        }
        "hook.output_budget" => {
            let config: BudgetConfig =
                serde_json::from_value(registry.module_config().clone()).map_err(error)?;
            if config.max_bytes == 0 || config.head_bytes > config.max_bytes {
                return Err(error(
                    "hook.output_budget requires max_bytes > 0 and head_bytes <= max_bytes",
                ));
            }
            Box::new(OutputBudget(config))
        }
        _ => return Err(error(format!("unknown hook module {module_id}"))),
    };
    registry.register_hook(module_id.to_owned(), hook)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::{
        contracts::ExecutionAttribution,
        domain::{ToolCall, ToolResult, new_call_id, new_execution_id},
    };
    #[test]
    fn budget_preserves_utf8_and_strict_byte_limit() {
        let id = new_call_id();
        let input = HookInput {
            conversation: None,
            event: HookEvent::AfterTool {
                call: ToolCall::new(id.clone(), "read", serde_json::json!({})),
                result: ToolResult::ok(id, "абвгдеёж"),
            },
            attribution: ExecutionAttribution::detached(new_execution_id()),
            cwd: "/tmp".into(),
        };
        let response = OutputBudget(BudgetConfig {
            max_bytes: 7,
            head_bytes: 3,
        })
        .invoke_json(serde_json::to_string(&input).unwrap())
        .unwrap();
        let HookResponse::ToolOutput { output } = serde_json::from_str(&response).unwrap() else {
            panic!("expected trimmed output")
        };
        assert_eq!(output, "аёж");
        assert!(output.len() <= 7);
    }
    #[test]
    fn config_rejects_unknown_fields_and_missing_required_values() {
        assert!(
            serde_json::from_value::<InstructionsConfig>(
                serde_json::json!({"text":"x","placement":"append","extra":true})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<BudgetConfig>(serde_json::json!({"max_bytes":5})).is_err()
        );
    }
}
