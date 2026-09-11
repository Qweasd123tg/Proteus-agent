//! Application content for human decisions. A2A still owns task/message wire
//! envelopes; these parts never execute tools or bypass the existing policy.
use a2a::{A2AError, Message, Part, PartContent, Task, TaskStatus};
use a2a_server::ServiceParams;
use serde::Deserialize;

use crate::{
    app_server::{AppPendingRequests, AppServerHandle},
    contracts::{ApprovalCacheScope, UserInputResponse},
};

pub(super) const INTERACTION_EXTENSION: &str = "urn:proteus:a2a:interaction:v1";

pub(super) fn enabled(params: &ServiceParams) -> bool {
    params
        .get("a2a-extensions")
        .into_iter()
        .flatten()
        .flat_map(|value| value.split(','))
        .any(|uri| uri.trim() == INTERACTION_EXTENSION)
}

pub(super) fn project_status(mut status: TaskStatus, enabled: bool) -> TaskStatus {
    if !enabled && let Some(message) = status.message.as_mut() {
        project_message(message);
    }
    status
}

pub(super) fn project_task(mut task: Task, enabled: bool) -> Task {
    task.status = project_status(task.status, enabled);
    if !enabled && let Some(history) = task.history.as_mut() {
        for message in history {
            project_message(message);
        }
    }
    task
}

fn project_message(message: &mut Message) {
    if message
        .extensions
        .as_ref()
        .is_some_and(|items| items.iter().any(|uri| uri == INTERACTION_EXTENSION))
    {
        message.extensions = None;
        message.parts.retain(|part| part.as_text().is_some());
        if message.parts.is_empty() {
            message
                .parts
                .push(Part::text("Proteus interaction response"));
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Response {
    Approval {
        approval_id: String,
        approved: bool,
        note: Option<String>,
    },
    UserInput {
        request_id: String,
        response: UserInputResponse,
    },
}

pub(super) fn pending_parts(pending: &AppPendingRequests) -> Option<Vec<Part>> {
    if pending.approvals.is_empty() && pending.user_inputs.is_empty() {
        return None;
    }
    Some(vec![
        Part::text("Proteus is waiting for approval or user input."),
        Part::data(serde_json::json!({
            "approvals": pending.approvals,
            "user_inputs": pending.user_inputs,
        })),
    ])
}

pub(super) async fn respond(server: &AppServerHandle, message: &Message) -> Result<(), A2AError> {
    if !message
        .extensions
        .as_ref()
        .is_some_and(|items| items.iter().any(|uri| uri == INTERACTION_EXTENSION))
    {
        return Err(A2AError::unsupported_operation(
            "input-required needs urn:proteus:a2a:interaction:v1",
        ));
    }
    let [part] = message.parts.as_slice() else {
        return Err(A2AError::invalid_params(
            "one interaction data part is required",
        ));
    };
    let PartContent::Data(data) = &part.content else {
        return Err(A2AError::content_type_not_supported());
    };
    let response: Response = serde_json::from_value(data.clone())
        .map_err(|error| A2AError::invalid_params(error.to_string()))?;
    match response {
        Response::Approval {
            approval_id,
            approved,
            note,
        } => {
            server
                .respond_approval(&approval_id, approved, note, ApprovalCacheScope::default())
                .await
        }
        Response::UserInput {
            request_id,
            response,
        } => server.respond_user_input(&request_id, response).await,
    }
    .map_err(|error| A2AError::invalid_params(error.to_string()))
}
