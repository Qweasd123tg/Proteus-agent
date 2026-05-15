//! Claude-like behavior pack.
//!
//! This pack deliberately stays inside existing slots:
//! - `workflow`: phased explore/edit/verify loop.
//! - `tool_exposure`: phase-aware subset of policy-visible tools.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

mod tools;
mod util;

use agent_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    contracts::{CompactionInput, ToolExposureInput, ToolExposureOutput, ToolExposureRequest},
    domain::{
        AgentOutput, ContextBundle, Event, TokenUsageCategory, TokenUsageSnapshot,
        TokenUsageSource, ToolCall, ToolChoice, ToolResult, ToolSpec,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        InstructionBlock, InstructionKind, MessageRole, TokenUsage,
    },
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginTool_TO,
        PluginToolExposure, PluginToolExposure_TO, PluginToolExposureError, PluginToolObject,
        PluginWorkflow, PluginWorkflow_TO, PluginWorkflowError, PluginWorkflowHostMut,
        PluginWorkflowInput, PluginWorkflowOutput, ToolExposureObject, WorkflowObject,
    },
};
use serde_json::{Value, json};
use tools::{BashTool, EditTool, GlobTool, GrepTool, ReadTool, TodoWriteTool, WriteTool};

const WORKFLOW_ID: &str = "claude.explore_edit_verify";
const TOOL_EXPOSURE_ID: &str = "claude_phased";
const MAX_TOOL_ROUNDS: usize = 10;

const SYSTEM_INSTRUCTIONS: &str = "\
You are a Claude-Code-like coding agent running inside Modular Agent. \
Be practical and direct. For code work, first understand the repository shape, \
then edit narrowly, then verify with the most relevant available check. \
Use tools only when needed and only from the current tool list. Do not claim a \
test/check passed unless you actually ran it. Do not create files or run broad \
commands just to demonstrate capability. If the user is only testing tools, run \
small targeted checks and report exactly what worked. Follow repository \
instructions over this behavior pack when they conflict.";

const EXPLORE_INSTRUCTIONS: &str = "\
Explore phase: orient with read-only tools. Prefer list_dir, read_file, grep, \
and search. Do not edit yet unless the user explicitly gave a tiny direct edit \
and the relevant file is already known. Use AskUserQuestion/request_user_input \
for broad or underspecified tasks before writing a plan; ask one focused \
multiple-choice question at a time when later questions depend on earlier \
answers. Only write the plan once material requirements are clear or the user \
explicitly skips the interview. Do not ask whether the plan is approved.";

const EDIT_INSTRUCTIONS: &str = "\
Edit phase: make the smallest coherent change using apply_patch or write_file. \
Inspect files before modifying them. Keep unrelated refactors out. After an \
edit, move toward verification instead of continuing to browse.";

const VERIFY_INSTRUCTIONS: &str = "\
Verify phase: run the most relevant available check. Prefer focused tests or \
project-native validation over broad expensive commands. If no verification is \
possible, say that explicitly in the final response. Do not edit unless the \
verification result shows a concrete issue.";

const FINAL_INSTRUCTIONS: &str = "\
Final phase: answer concisely. Mention what changed or what was found, list \
verification that actually ran, and call out remaining risk. Do not ask for \
tools in this phase.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Explore,
    Edit,
    Verify,
}

impl Phase {
    fn reason(self) -> &'static str {
        match self {
            Phase::Explore => "claude.explore",
            Phase::Edit => "claude.edit",
            Phase::Verify => "claude.verify",
        }
    }

    fn instructions(self) -> &'static str {
        match self {
            Phase::Explore => EXPLORE_INSTRUCTIONS,
            Phase::Edit => EDIT_INSTRUCTIONS,
            Phase::Verify => VERIFY_INSTRUCTIONS,
        }
    }
}

struct ClaudeWorkflow;
struct ClaudePhasedToolExposure;

impl PluginWorkflow for ClaudeWorkflow {
    fn run_json(
        &self,
        input_json: RString,
        host: &mut PluginWorkflowHostMut<'_>,
    ) -> RResult<RString, PluginWorkflowError> {
        let input: PluginWorkflowInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return workflow_err(error),
        };

        match run_workflow(input, host) {
            Ok(output) => match serde_json::to_string(&output) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => workflow_err(error),
            },
            Err(error) => RResult::RErr(error),
        }
    }
}

impl PluginToolExposure for ClaudePhasedToolExposure {
    fn select_json(&self, input_json: RString) -> RResult<RString, PluginToolExposureError> {
        let input: ToolExposureInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => return tool_exposure_err(error),
        };
        let tools = select_tools(input);
        let mut output = ToolExposureOutput::new(tools);
        output.metadata = json!({
            "module_id": TOOL_EXPOSURE_ID,
        });
        match serde_json::to_string(&output) {
            Ok(json) => RResult::ROk(RString::from(json)),
            Err(error) => tool_exposure_err(error),
        }
    }
}

fn run_workflow(
    input: PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
) -> Result<PluginWorkflowOutput, PluginWorkflowError> {
    emit_event(
        host,
        &Event::TaskReceived {
            task: input.task.clone(),
        },
    )?;

    let bundle = build_context(host, &input)?;
    emit_event(
        host,
        &Event::ContextBuilt {
            chunks: bundle.chunks.len(),
            token_estimate: bundle.token_estimate,
        },
    )?;

    let context_chunks = bundle.chunks.len();
    let context_token_estimate = bundle.token_estimate;
    let mut persistent_messages = input.history.clone();
    let user_message = CanonicalMessage::text(MessageRole::User, input.task.text.clone());
    persistent_messages.push(user_message.clone());

    let mut model_messages = persistent_messages.clone();
    for chunk in bundle.chunks {
        model_messages.push(
            CanonicalMessage::new(MessageRole::User, vec![ContentPart::Context { chunk }])
                .with_name("context"),
        );
    }

    let mut phase = Phase::Explore;
    let mut phases = vec![phase.reason()];
    let mut tool_round_limit_reached = true;
    for _round in 0..MAX_TOOL_ROUNDS {
        let request = request_from_state(&input, host, &model_messages, phase)?;
        emit_event(
            host,
            &Event::ModelRequestPrepared {
                model: request.model.clone(),
            },
        )?;
        let response = complete_model(host, &request, phase.reason())?;
        emit_event(
            host,
            &Event::ModelResponseReceived {
                finish_reason: response.finish_reason.clone(),
            },
        )?;

        model_messages.push(with_workflow_phase(
            response.message.clone(),
            phase.reason(),
        ));
        let should_run_tools =
            response.finish_reason == FinishReason::ToolCalls && !response.tool_calls.is_empty();
        if should_run_tools {
            persistent_messages.push(response.message);
        }
        if !should_run_tools {
            tool_round_limit_reached = false;
            break;
        }

        let mut next_phase = phase;
        for call in response.tool_calls {
            next_phase = next_phase_after_tool(next_phase, &call);
            let result = execute_tool(host, &input, &call)?;
            let call_id = result.call_id.clone();
            if !result.ok {
                next_phase = Phase::Edit;
            }
            let tool_result_message =
                CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                    .with_tool_call_id(call_id);
            model_messages.push(tool_result_message.clone());
            persistent_messages.push(tool_result_message);
        }
        phase = next_phase;
        phases.push(phase.reason());
    }

    let final_response = final_model_response(&input, host, &model_messages)?;
    model_messages.push(final_response.message.clone());
    persistent_messages.push(final_response.message.clone());
    let output = AgentOutput::new(
        message_text(&final_response.message),
        output_metadata(
            &input,
            &model_messages,
            context_chunks,
            context_token_estimate,
            json!({
                "max_tool_rounds": MAX_TOOL_ROUNDS,
                "tool_round_limit_reached": tool_round_limit_reached,
                "phases": phases,
            }),
        ),
    );
    emit_event(
        host,
        &Event::TurnFinished {
            output: output.clone(),
        },
    )?;
    Ok(PluginWorkflowOutput {
        output,
        messages: persistent_messages,
    })
}

fn next_phase_after_tool(current: Phase, call: &ToolCall) -> Phase {
    match call.name.as_str() {
        "Edit" | "Write" | "apply_patch" | "write_file" => Phase::Verify,
        "Bash" | "shell" if current == Phase::Verify => Phase::Verify,
        "Bash" | "shell" => current,
        "Read" | "Glob" | "Grep" | "read_file" | "list_dir" | "grep" | "search" => {
            if current == Phase::Explore {
                Phase::Edit
            } else {
                current
            }
        }
        _ => current,
    }
}

fn request_from_state(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
    phase: Phase,
) -> Result<CanonicalModelRequest, PluginWorkflowError> {
    let tools = phase_tools(host, input, phase)?;
    let messages = compact_messages(input, host, messages, phase.reason())?;
    Ok(
        CanonicalModelRequest::new(input.runtime.model_ref.clone(), messages)
            .with_instructions(vec![
                InstructionBlock::new(InstructionKind::System, SYSTEM_INSTRUCTIONS, 100),
                InstructionBlock::new(InstructionKind::Developer, phase.instructions(), 90),
            ])
            .with_tools(tools)
            .with_reasoning(input.runtime.reasoning.clone()),
    )
}

fn final_model_response(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
) -> Result<CanonicalModelResponse, PluginWorkflowError> {
    let messages = compact_messages(input, host, messages, "claude.final")?;
    let mut request = CanonicalModelRequest::new(input.runtime.model_ref.clone(), messages)
        .with_instructions(vec![
            InstructionBlock::new(InstructionKind::System, SYSTEM_INSTRUCTIONS, 100),
            InstructionBlock::new(InstructionKind::Developer, FINAL_INSTRUCTIONS, 90),
        ])
        .with_tool_choice(ToolChoice::None)
        .with_reasoning(input.runtime.reasoning.clone());
    request.tools.clear();
    emit_event(
        host,
        &Event::ModelRequestPrepared {
            model: request.model.clone(),
        },
    )?;
    let response = complete_model(host, &request, "claude.final")?;
    emit_event(
        host,
        &Event::ModelResponseReceived {
            finish_reason: response.finish_reason.clone(),
        },
    )?;
    Ok(response)
}

fn phase_tools(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    phase: Phase,
) -> Result<Vec<ToolSpec>, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let request = ToolExposureRequest::new(input.task.clone())
        .with_reason(phase.reason())
        .with_query(input.task.text.clone());
    let request_json = to_json_string(&request)?;
    let tools_json = match host.select_tools_json(RString::from(request_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    let output: ToolExposureOutput = from_json_string(tools_json.as_str())?;
    Ok(output.tools)
}

fn select_tools(input: ToolExposureInput) -> Vec<ToolSpec> {
    let reason = input.request.reason.as_deref().unwrap_or_default();
    let preferred = if reason.contains("verify") {
        &[
            "Bash",
            "shell",
            "Read",
            "read_file",
            "Grep",
            "grep",
            "search",
            "Glob",
            "list_dir",
            "Edit",
            "apply_patch",
        ][..]
    } else if reason.contains("edit") {
        &[
            "Read",
            "read_file",
            "Grep",
            "grep",
            "search",
            "Glob",
            "list_dir",
            "Edit",
            "apply_patch",
            "Write",
            "write_file",
        ][..]
    } else {
        &[
            "Read",
            "read_file",
            "Glob",
            "list_dir",
            "Grep",
            "grep",
            "search",
            "AskUserQuestion",
            "request_user_input",
            "TodoWrite",
        ][..]
    };

    let max_tools = input.request.max_tools.unwrap_or(preferred.len());
    let mut selected = Vec::new();
    for name in preferred {
        if let Some(tool) = input.candidates.iter().find(|tool| tool.name == *name) {
            selected.push(tool.clone());
        }
        if selected.len() >= max_tools {
            break;
        }
    }
    selected
}

fn compact_messages(
    input: &PluginWorkflowInput,
    host: &mut PluginWorkflowHostMut<'_>,
    messages: &[CanonicalMessage],
    reason: &str,
) -> Result<Vec<CanonicalMessage>, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let compaction_input = CompactionInput::new(
        input.task.clone(),
        input.runtime.model_ref.clone(),
        messages.to_vec(),
    )
    .with_reason(reason)
    .with_token_estimate(estimate_message_tokens(messages));
    let input_json = to_json_string(&compaction_input)?;
    let output_json = match host.compact_history_json(RString::from(input_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    let output: agent_contracts::contracts::CompactionOutput =
        from_json_string(output_json.as_str())?;
    if output.messages.is_empty() && !messages.is_empty() {
        return Err(PluginWorkflowError::new(
            "compactor returned empty messages for non-empty history",
        ));
    }
    Ok(output.messages)
}

fn build_context(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
) -> Result<ContextBundle, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let task_json = to_json_string(&input.task)?;
    let bundle_json = match host.build_context_json(RString::from(task_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    from_json_string(bundle_json.as_str())
}

fn complete_model(
    host: &mut PluginWorkflowHostMut<'_>,
    request: &CanonicalModelRequest,
    phase: &str,
) -> Result<CanonicalModelResponse, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let request_json = to_json_string(request)?;
    let response_json = match host.complete_model_json(RString::from(request_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    let response: CanonicalModelResponse = from_json_string(response_json.as_str())?;
    emit_token_usage(host, request, response.usage.clone(), phase)?;
    Ok(response)
}

fn execute_tool(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
) -> Result<ToolResult, PluginWorkflowError> {
    ensure_not_cancelled(host)?;
    let task_json = to_json_string(&input.task)?;
    let call_json = to_json_string(call)?;
    let result_json = match host
        .execute_tool_json(RString::from(task_json), RString::from(call_json))
    {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(PluginWorkflowError::new(error.message.into_string())),
    };
    from_json_string(result_json.as_str())
}

fn ensure_not_cancelled(host: &mut PluginWorkflowHostMut<'_>) -> Result<(), PluginWorkflowError> {
    match host.is_cancelled() {
        RResult::ROk(false) => Ok(()),
        RResult::ROk(true) => Err(PluginWorkflowError::new("turn canceled by client")),
        RResult::RErr(error) => Err(PluginWorkflowError::new(error.message.into_string())),
    }
}

fn emit_event(
    host: &mut PluginWorkflowHostMut<'_>,
    event: &Event,
) -> Result<(), PluginWorkflowError> {
    let event_json = to_json_string(event)?;
    match host.emit_event_json(RString::from(event_json)) {
        RResult::ROk(()) => Ok(()),
        RResult::RErr(error) => Err(PluginWorkflowError::new(error.message.into_string())),
    }
}

fn emit_token_usage(
    host: &mut PluginWorkflowHostMut<'_>,
    request: &CanonicalModelRequest,
    actual: Option<TokenUsage>,
    phase: &str,
) -> Result<(), PluginWorkflowError> {
    let usage = request_token_usage_snapshot(request, actual, phase);
    emit_event(host, &Event::TokenUsageUpdated { usage })
}

fn request_token_usage_snapshot(
    request: &CanonicalModelRequest,
    actual: Option<TokenUsage>,
    phase: &str,
) -> TokenUsageSnapshot {
    let categories = estimate_request_categories(request);
    let estimated_input_tokens = categories.iter().map(|category| category.tokens).sum();
    let source = if actual.is_some() {
        TokenUsageSource::Mixed
    } else {
        TokenUsageSource::Estimated
    };
    TokenUsageSnapshot::new(request.model.clone(), estimated_input_tokens, categories)
        .with_phase(phase)
        .with_max_input_tokens(request.limits.max_input_tokens)
        .with_actual(actual)
        .with_source(source)
}

fn estimate_request_categories(request: &CanonicalModelRequest) -> Vec<TokenUsageCategory> {
    let instruction_bytes = request
        .instructions
        .iter()
        .map(|instruction| instruction.text.len() + 8)
        .sum::<usize>();
    let message_bytes = request.messages.iter().map(message_text_len).sum::<usize>();
    let tool_schema_bytes = request
        .tools
        .iter()
        .map(|tool| {
            serde_json::to_string(tool)
                .map(|json| json.len())
                .unwrap_or(0)
        })
        .sum::<usize>();

    [
        ("instructions", instruction_bytes),
        ("messages", message_bytes),
        ("tool_schemas", tool_schema_bytes),
    ]
    .into_iter()
    .filter_map(|(name, bytes)| {
        let tokens = estimate_tokens_from_bytes(bytes);
        (tokens > 0).then(|| TokenUsageCategory::new(name, tokens))
    })
    .collect()
}

fn message_text_len(message: &CanonicalMessage) -> usize {
    message.parts.iter().map(part_text_len).sum::<usize>() + 4
}

fn part_text_len(part: &ContentPart) -> usize {
    match part {
        ContentPart::Text { text }
        | ContentPart::ReasoningSummary { text }
        | ContentPart::Reasoning { text, .. } => text.len(),
        ContentPart::Context { chunk } => {
            chunk.source.len()
                + chunk
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string().len())
                    .unwrap_or_default()
                + chunk.content.len()
                + chunk.metadata.to_string().len()
        }
        ContentPart::ToolCall { call } => call.name.len() + call.args.to_string().len(),
        ContentPart::ToolResult { result } => {
            result.output.len()
                + result.error.as_deref().unwrap_or_default().len()
                + result.metadata.to_string().len()
        }
        ContentPart::FileRef { content, .. } => content.as_deref().unwrap_or_default().len(),
        ContentPart::Patch { patch } => patch.content.len(),
        _ => 0,
    }
}

fn estimate_tokens_from_bytes(bytes: usize) -> u32 {
    if bytes == 0 {
        0
    } else {
        (bytes / 4).max(1) as u32
    }
}

fn estimate_message_tokens(messages: &[CanonicalMessage]) -> Option<u32> {
    Some((messages.iter().map(message_text_len).sum::<usize>() / 4 + messages.len()).max(1) as u32)
}

fn with_workflow_phase(mut message: CanonicalMessage, phase: &str) -> CanonicalMessage {
    match &mut message.metadata {
        Value::Object(metadata) => {
            metadata.insert("workflow_module".to_owned(), WORKFLOW_ID.into());
            metadata.insert("workflow_phase".to_owned(), phase.into());
        }
        _ => {
            message.metadata = json!({
                "workflow_module": WORKFLOW_ID,
                "workflow_phase": phase,
            });
        }
    }
    message
}

fn output_metadata(
    input: &PluginWorkflowInput,
    messages: &[CanonicalMessage],
    context_chunks: usize,
    context_token_estimate: Option<u32>,
    extra: Value,
) -> Value {
    let mut metadata = json!({
        "session_id": input.runtime.session_id,
        "thread_id": input.runtime.thread_id,
        "turn_id": input.runtime.turn_id,
        "model": {
            "provider": input.runtime.model_ref.provider.clone(),
            "name": input.runtime.model_ref.model.clone(),
        },
        "context": {
            "chunks": context_chunks,
            "token_estimate": estimate_message_tokens(messages).or(context_token_estimate),
            "initial_token_estimate": context_token_estimate,
        },
        "workflow": {
            "source": "plugin",
            "module_id": WORKFLOW_ID,
            "style": "claude_code_like",
        },
    });
    if let (Value::Object(metadata), Value::Object(extra)) = (&mut metadata, extra) {
        metadata.extend(extra);
    }
    metadata
}

fn message_text(message: &CanonicalMessage) -> String {
    let text = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        "<empty model response>".to_owned()
    } else {
        text
    }
}

fn to_json_string<T: serde::Serialize>(value: &T) -> Result<String, PluginWorkflowError> {
    serde_json::to_string(value).map_err(|error| PluginWorkflowError::new(error.to_string()))
}

fn from_json_string<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, PluginWorkflowError> {
    serde_json::from_str(value).map_err(|error| PluginWorkflowError::new(error.to_string()))
}

fn workflow_err<T>(error: impl ToString) -> RResult<T, PluginWorkflowError> {
    RResult::RErr(PluginWorkflowError::new(error.to_string()))
}

fn tool_exposure_err<T>(error: impl ToString) -> RResult<T, PluginToolExposureError> {
    RResult::RErr(PluginToolExposureError::new(error.to_string()))
}

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    for tool in [
        PluginTool_TO::from_value(ReadTool, TD_Opaque),
        PluginTool_TO::from_value(WriteTool, TD_Opaque),
        PluginTool_TO::from_value(EditTool, TD_Opaque),
        PluginTool_TO::from_value(GrepTool, TD_Opaque),
        PluginTool_TO::from_value(GlobTool, TD_Opaque),
        PluginTool_TO::from_value(BashTool, TD_Opaque),
        PluginTool_TO::from_value(TodoWriteTool, TD_Opaque),
    ] {
        let tool: PluginToolObject = tool;
        if let RResult::RErr(err) = registry.register_tool(tool) {
            return RResult::RErr(err);
        }
    }

    let workflow: WorkflowObject = PluginWorkflow_TO::from_value(ClaudeWorkflow, TD_Opaque);
    if let RResult::RErr(err) = registry.register_workflow(RString::from(WORKFLOW_ID), workflow) {
        return RResult::RErr(err);
    }

    let exposure: ToolExposureObject =
        PluginToolExposure_TO::from_value(ClaudePhasedToolExposure, TD_Opaque);
    registry.register_tool_exposure(RString::from(TOOL_EXPOSURE_ID), exposure)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("claude-pack"),
        description: RStr::from_str(
            "Claude-Code-like behavior pack with phased workflow, tool exposure, and tool aliases",
        ),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_contracts::domain::{AgentTask, ToolSafety, ToolSpec};

    fn spec(name: &str, safety: ToolSafety) -> ToolSpec {
        ToolSpec::new(name, name, json!({}), safety)
    }

    fn input(reason: &str) -> ToolExposureInput {
        ToolExposureInput::new(
            ToolExposureRequest::new(AgentTask::new("change code", ".".into())).with_reason(reason),
            vec![
                spec("Bash", ToolSafety::RunsCommands),
                spec("shell", ToolSafety::RunsCommands),
                spec("Write", ToolSafety::WritesFiles),
                spec("write_file", ToolSafety::WritesFiles),
                spec("Edit", ToolSafety::WritesFiles),
                spec("apply_patch", ToolSafety::WritesFiles),
                spec("Read", ToolSafety::ReadOnly),
                spec("read_file", ToolSafety::ReadOnly),
                spec("Glob", ToolSafety::ReadOnly),
                spec("list_dir", ToolSafety::ReadOnly),
                spec("Grep", ToolSafety::ReadOnly),
                spec("grep", ToolSafety::ReadOnly),
                spec("search", ToolSafety::ReadOnly),
                spec("AskUserQuestion", ToolSafety::ReadOnly),
                spec("request_user_input", ToolSafety::ReadOnly),
                spec("TodoWrite", ToolSafety::ReadOnly),
                spec("remember_fact", ToolSafety::WritesFiles),
            ],
        )
    }

    #[test]
    fn explore_exposes_only_read_tools() {
        let tools = select_tools(input("claude.explore"));
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "Read",
                "read_file",
                "Glob",
                "list_dir",
                "Grep",
                "grep",
                "search",
                "AskUserQuestion",
                "request_user_input",
                "TodoWrite"
            ]
        );
    }

    #[test]
    fn edit_exposes_patch_and_write_but_not_shell() {
        let tools = select_tools(input("claude.edit"));
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "Read",
                "read_file",
                "Grep",
                "grep",
                "search",
                "Glob",
                "list_dir",
                "Edit",
                "apply_patch",
                "Write",
                "write_file"
            ]
        );
    }

    #[test]
    fn verify_exposes_shell_first() {
        let tools = select_tools(input("claude.verify"));
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "Bash",
                "shell",
                "Read",
                "read_file",
                "Grep",
                "grep",
                "search",
                "Glob",
                "list_dir",
                "Edit",
                "apply_patch"
            ]
        );
    }

    #[test]
    fn tool_call_transitions_toward_verify_after_edit() {
        let call = ToolCall::new("call-1", "apply_patch", json!({}));
        assert_eq!(next_phase_after_tool(Phase::Edit, &call), Phase::Verify);
        let call = ToolCall::new("call-2", "Edit", json!({}));
        assert_eq!(next_phase_after_tool(Phase::Edit, &call), Phase::Verify);
    }
}
