use a2a::{A2AError, Message, Part, Role, SendMessageRequest, Task};
use a2a_server::ServiceParams;

pub(super) fn service(params: &ServiceParams) -> Result<(), A2AError> {
    if let Some(versions) = params.get("a2a-version") {
        if versions.len() != 1 || versions[0] != a2a::VERSION {
            return Err(A2AError::version_not_supported(&versions.join(",")));
        }
    }
    Ok(())
}

pub(super) fn tenant(value: &Option<String>) -> Result<(), A2AError> {
    if value.is_some() {
        return Err(A2AError::unsupported_operation("tenants are not supported"));
    }
    Ok(())
}

pub(super) fn history_length(value: Option<i32>) -> Result<(), A2AError> {
    if value.is_some_and(|length| length < 0) {
        return Err(A2AError::invalid_params(
            "historyLength must be nonnegative",
        ));
    }
    Ok(())
}

pub(super) fn request(req: &SendMessageRequest) -> Result<(), A2AError> {
    tenant(&req.tenant)?;
    if req.message.message_id.is_empty()
        || req.message.role != Role::User
        || req.message.parts.is_empty()
    {
        return Err(A2AError::invalid_params(
            "a nonempty user message with messageId is required",
        ));
    }
    if req.message.context_id.as_deref() == Some("") || req.message.task_id.as_deref() == Some("") {
        return Err(A2AError::invalid_params(
            "taskId and contextId must not be empty",
        ));
    }
    if req.message.extensions.as_ref().is_some_and(|items| {
        items
            .iter()
            .any(|uri| uri != super::interaction::INTERACTION_EXTENSION)
    }) {
        return Err(A2AError::unsupported_operation(
            "unsupported message extension",
        ));
    }
    if let Some(config) = &req.configuration {
        history_length(config.history_length)?;
        if config.task_push_notification_config.is_some() {
            return Err(A2AError::push_notification_not_supported());
        }
        if config.accepted_output_modes.as_ref().is_some_and(|modes| {
            !modes.is_empty() && !modes.iter().any(|mode| mode == "text/plain")
        }) {
            return Err(A2AError::content_type_not_supported());
        }
    }
    Ok(())
}

pub(super) fn text(message: &Message) -> Result<String, A2AError> {
    let parts = message
        .parts
        .iter()
        .map(Part::as_text)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(A2AError::content_type_not_supported)?;
    let text = parts.join("\n");
    if text.trim().is_empty() || text.len() > 16_000 {
        return Err(A2AError::invalid_params("text must contain 1..16000 bytes"));
    }
    Ok(text)
}

pub(super) fn trim_history(mut task: Task, length: Option<i32>) -> Task {
    if let (Some(length), Some(history)) = (length, task.history.as_mut()) {
        let remove = history.len().saturating_sub(length as usize);
        history.drain(..remove);
    }
    task
}
