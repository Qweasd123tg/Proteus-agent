//! Ordered, execution-bound contributions at typed runtime boundaries.
use super::{ExecutionAttribution, ModelCallOrigin};
use crate::{
    domain::{AgentOutput, AgentTask, ToolCall, ToolResult, ToolSpec, validate_tool_call_args},
    model_standard::{CanonicalMessage, CanonicalModelRequest, InstructionBlock},
};
use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const PROCESS_HOOK_CONTRACT_VERSION: &str = "v3";
pub const PROCESS_HOOK_INVOKE_METHOD: &str = "hook.invoke";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HookInput {
    pub event: HookEvent,
    pub attribution: ExecutionAttribution,
    pub cwd: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookEvent {
    TurnStarted {
        task: AgentTask,
        history: Vec<CanonicalMessage>,
    },
    BeforeModel {
        origin: ModelCallOrigin,
        request: CanonicalModelRequest,
    },
    BeforeTool {
        call: ToolCall,
        spec: Option<ToolSpec>,
        blocked: Option<String>,
    },
    AfterTool {
        call: ToolCall,
        result: ToolResult,
    },
    BeforeStop {
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        output: AgentOutput,
        attempt: u32,
        continuation: Option<String>,
    },
    TurnSettled {
        status: HookTurnStatus,
        output: Option<AgentOutput>,
        error: Option<String>,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookTurnStatus {
    Success,
    Error,
    Canceled,
    Timeout,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookResponse {
    Continue,
    ModelContext {
        messages: Vec<CanonicalMessage>,
        instructions: Vec<InstructionBlock>,
    },
    ToolArguments {
        args: serde_json::Value,
    },
    ContinueTurn {
        reason: String,
    },
    BlockTool {
        reason: String,
    },
    ToolOutput {
        output: String,
    },
}
impl<'de> Deserialize<'de> for HookResponse {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Continue {},
            ModelContext {
                messages: Vec<CanonicalMessage>,
                instructions: Vec<InstructionBlock>,
            },
            ToolArguments {
                args: serde_json::Value,
            },
            ContinueTurn {
                reason: String,
            },
            BlockTool {
                reason: String,
            },
            ToolOutput {
                output: String,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Continue {} => Self::Continue,
            Wire::ModelContext {
                messages,
                instructions,
            } => Self::ModelContext {
                messages,
                instructions,
            },
            Wire::ToolArguments { args } => Self::ToolArguments { args },
            Wire::ContinueTurn { reason } => Self::ContinueTurn { reason },
            Wire::BlockTool { reason } => Self::BlockTool { reason },
            Wire::ToolOutput { output } => Self::ToolOutput { output },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessHookResponse {
    pub result: HookResponse,
}

pub fn apply_hook_response(event: &HookEvent, response: &HookResponse) -> Result<HookEvent> {
    let mut event = event.clone();
    match (&mut event, response) {
        (_, HookResponse::Continue) => {}
        (
            HookEvent::BeforeModel { request, .. },
            HookResponse::ModelContext {
                messages,
                instructions,
            },
        ) => {
            validate_model_context(&request.messages, messages)?;
            request.messages = messages.clone();
            request.instructions = instructions.clone();
        }
        (HookEvent::BeforeTool { call, spec, .. }, HookResponse::ToolArguments { args }) => {
            call.args = args.clone();
            call.raw_arguments = None;
            if let Some(spec) = spec
                && let Some(error) = validate_tool_call_args(call, spec)
            {
                bail!("invalid hook tool arguments: {error}");
            }
        }
        (HookEvent::BeforeStop { continuation, .. }, HookResponse::ContinueTurn { reason })
            if !reason.trim().is_empty() =>
        {
            *continuation = Some(reason.clone());
        }
        (HookEvent::BeforeTool { blocked, .. }, HookResponse::BlockTool { reason })
            if !reason.trim().is_empty() =>
        {
            *blocked = Some(reason.clone());
        }
        (HookEvent::AfterTool { result, .. }, HookResponse::ToolOutput { output }) => {
            result.output = output.clone()
        }
        _ => bail!("hook response is not allowed for this event, or block reason is blank"),
    }
    Ok(event)
}
fn validate_model_context(
    original: &[CanonicalMessage],
    messages: &[CanonicalMessage],
) -> Result<()> {
    use std::collections::{HashMap, HashSet};
    let original_parts = original
        .iter()
        .flat_map(|m| &m.parts)
        .map(|p| (p.part_id, p))
        .collect::<HashMap<_, _>>();
    let original_messages = original
        .iter()
        .map(|m| (m.id, m))
        .collect::<HashMap<_, _>>();
    let mut message_ids = HashSet::new();
    let mut parts = HashMap::new();
    for message in messages {
        if !message_ids.insert(message.id) {
            bail!("hook model context has duplicate message id {}", message.id);
        }
        if let Some(previous) = original_messages.get(&message.id) {
            if message.role != previous.role
                || message.phase != previous.phase
                || message.name != previous.name
                || message.tool_call_id != previous.tool_call_id
                || message.metadata != previous.metadata
            {
                bail!(
                    "hook model context changes existing message identity {}",
                    message.id
                );
            }
        }
        for part in &message.parts {
            if let Some(previous) = original_parts.get(&part.part_id) {
                if *previous != part {
                    bail!("hook model context changes immutable part {}", part.part_id);
                }
            }
            if let Some(previous) = parts.insert(part.part_id, part) {
                if previous != part {
                    bail!(
                        "hook model context has conflicting part id {}",
                        part.part_id
                    );
                }
            }
        }
    }
    Ok(())
}

#[async_trait]
pub trait HookHandler: Send + Sync {
    async fn invoke(
        &self,
        input: HookInput,
        cancellation: super::CancellationToken,
    ) -> Result<HookResponse>;
}
#[async_trait]
pub trait ExecutionHooks: Send + Sync {
    fn is_active(&self) -> bool {
        true
    }
    async fn apply(&self, input: HookInput) -> Result<HookEvent>;
}
#[derive(Debug, Default)]
pub struct NoExecutionHooks;
#[async_trait]
impl ExecutionHooks for NoExecutionHooks {
    fn is_active(&self) -> bool {
        false
    }
    async fn apply(&self, input: HookInput) -> Result<HookEvent> {
        Ok(input.event)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HookTrace {
    pub input: HookInput,
    pub steps: Vec<HookStep>,
    pub output: Option<HookEvent>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HookStep {
    pub module_id: String,
    pub outcome: HookStepOutcome,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookStepOutcome {
    Accepted { response: HookResponse },
    Failed { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ModelRef, ToolCall, ToolResult, new_call_id};
    #[test]
    fn responses_cannot_change_identity_or_status() {
        let id = new_call_id();
        let event = HookEvent::AfterTool {
            call: ToolCall::new(id.clone(), "read", serde_json::json!({})),
            result: ToolResult::error(id, "original"),
        };
        let transformed = apply_hook_response(
            &event,
            &HookResponse::ToolOutput {
                output: "redacted".into(),
            },
        )
        .unwrap();
        let HookEvent::AfterTool { call, mut result } = transformed else {
            panic!()
        };
        result.output = String::new();
        assert_eq!(HookEvent::AfterTool { call, result }, event);
        assert!(
            apply_hook_response(
                &event,
                &HookResponse::BlockTool {
                    reason: "no".into()
                }
            )
            .is_err()
        );
        let model = HookEvent::BeforeModel {
            origin: ModelCallOrigin::Direct,
            request: CanonicalModelRequest::new(ModelRef::new("fake", "x"), vec![]),
        };
        assert!(
            apply_hook_response(&model, &HookResponse::ToolOutput { output: "x".into() }).is_err()
        );
    }
    #[test]
    fn notification_and_blank_blocks_are_rejected_strictly() {
        let notification = HookEvent::TurnSettled {
            status: HookTurnStatus::Success,
            output: None,
            error: None,
        };
        assert!(
            apply_hook_response(
                &notification,
                &HookResponse::ModelContext {
                    messages: vec![],
                    instructions: vec![]
                }
            )
            .is_err()
        );
        let before = HookEvent::BeforeTool {
            call: ToolCall::new(new_call_id(), "read", serde_json::json!({})),
            spec: None,
            blocked: None,
        };
        assert!(
            apply_hook_response(
                &before,
                &HookResponse::BlockTool {
                    reason: "  ".into()
                }
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<HookResponse>(r#"{"action":"continue","extra":true}"#).is_err()
        );
    }
    #[test]
    fn model_context_cannot_rewrite_existing_canonical_part() {
        let original = CanonicalMessage::text(crate::model_standard::MessageRole::User, "original");
        let event = HookEvent::BeforeModel {
            origin: ModelCallOrigin::Direct,
            request: CanonicalModelRequest::new(ModelRef::new("fake", "x"), vec![original.clone()]),
        };
        let mut changed = original;
        changed.parts[0].payload = crate::model_standard::ContentPart::Text {
            text: "rewritten".into(),
        };
        assert!(
            apply_hook_response(
                &event,
                &HookResponse::ModelContext {
                    messages: vec![changed],
                    instructions: vec![]
                }
            )
            .unwrap_err()
            .to_string()
            .contains("immutable part")
        );
    }
}
