use proteus_contracts::{
    model_standard::{CanonicalMessage, CanonicalModelRequest},
    plugin::PluginWorkflowInput,
};
use serde_json::{Value, json};

use crate::token_accounting::estimate_message_tokens;

pub(crate) fn output_metadata(
    module_id: &str,
    input: &PluginWorkflowInput,
    messages: &[CanonicalMessage],
    context_chunks: usize,
    context_token_estimate: Option<u32>,
) -> Value {
    output_metadata_with_extra(
        module_id,
        input,
        messages,
        context_chunks,
        context_token_estimate,
        json!({}),
    )
}

pub(crate) fn output_metadata_with_extra(
    module_id: &str,
    input: &PluginWorkflowInput,
    messages: &[CanonicalMessage],
    context_chunks: usize,
    context_token_estimate: Option<u32>,
    extra: Value,
) -> Value {
    let token_estimate = estimate_message_tokens(messages).or(context_token_estimate);
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
            "token_estimate": token_estimate,
            "initial_token_estimate": context_token_estimate,
        },
        "workflow": {
            "source": "plugin",
            "module_id": module_id,
        },
    });

    if let (Value::Object(metadata), Value::Object(extra)) = (&mut metadata, extra) {
        metadata.extend(extra);
    }

    metadata
}

pub(crate) fn with_workflow_phase(
    mut message: CanonicalMessage,
    module_id: &str,
    phase: &str,
) -> CanonicalMessage {
    match &mut message.metadata {
        Value::Object(metadata) => {
            metadata.insert(
                "workflow_module".to_owned(),
                Value::String(module_id.to_owned()),
            );
            metadata.insert("workflow_phase".to_owned(), Value::String(phase.to_owned()));
        }
        Value::Null => {
            message.metadata = json!({
                "workflow_module": module_id,
                "workflow_phase": phase,
            });
        }
        other => {
            let previous = std::mem::replace(other, Value::Null);
            message.metadata = json!({
                "workflow_module": module_id,
                "workflow_phase": phase,
                "previous_metadata": previous,
            });
        }
    }
    message
}

pub(crate) fn insert_request_metadata_u32(
    request: &mut CanonicalModelRequest,
    key: &str,
    value: u32,
) {
    insert_request_metadata_value(request, key, json!(value));
}

pub(crate) fn insert_request_metadata_value(
    request: &mut CanonicalModelRequest,
    key: &str,
    value: Value,
) {
    match &mut request.metadata {
        Value::Object(metadata) => {
            metadata.insert(key.to_owned(), value);
        }
        Value::Null => {
            request.metadata = json!({ key: value });
        }
        other => {
            let previous = std::mem::replace(other, Value::Null);
            request.metadata = json!({
                key: value,
                "previous_metadata": previous,
            });
        }
    }
}

pub(crate) fn prompt_cache_key(
    input: &PluginWorkflowInput,
    request: &CanonicalModelRequest,
) -> String {
    let provider = sanitize_cache_key_component(&input.runtime.model_ref.provider);
    let model = sanitize_cache_key_component(&input.runtime.model_ref.model);
    let workspace_hash = stable_hash64(input.task.cwd.to_string_lossy().as_bytes());
    let prefix_hash = stable_prompt_prefix_hash(request);
    format!("proteus:{provider}:{model}:{workspace_hash:016x}:{prefix_hash:016x}")
}

fn stable_prompt_prefix_hash(request: &CanonicalModelRequest) -> u64 {
    let mut text = String::new();
    text.push_str("instructions\n");
    let mut instructions = request.instructions.clone();
    instructions.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| format!("{:?}", left.kind).cmp(&format!("{:?}", right.kind)))
            .then_with(|| left.text.cmp(&right.text))
    });
    for instruction in instructions {
        text.push_str(&format!(
            "{:?}\t{}\t{}\n",
            instruction.kind, instruction.priority, instruction.text
        ));
    }

    text.push_str("tools\n");
    let mut tools = request.tools.clone();
    tools.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.description.cmp(&right.description))
    });
    for tool in tools {
        text.push_str(
            &serde_json::to_string(&tool)
                .unwrap_or_else(|_| format!("{}\t{}", tool.name, tool.description)),
        );
        text.push('\n');
    }

    stable_hash64(text.as_bytes())
}

fn sanitize_cache_key_component(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    out.truncate(64);
    if out.is_empty() {
        "model".to_owned()
    } else {
        out
    }
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
