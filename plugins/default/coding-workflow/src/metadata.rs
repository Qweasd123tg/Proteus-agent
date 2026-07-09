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

/// Стабильный routing key provider prompt cache для одной durable session.
///
/// Это не fingerprint содержимого запроса: provider отдельно хеширует
/// фактический prefix и переиспользует только совпавшую часть. Если включать в
/// key tools/instructions, любое легитимное изменение prefix разбрасывает одну
/// conversation по разным cache buckets и убивает reuse последующих turn-ов.
pub(crate) fn prompt_cache_key(input: &PluginWorkflowInput) -> String {
    format!("proteus:session:{}", input.runtime.session_id)
}
