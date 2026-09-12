use super::{commands::command_response, sessions::server_for_session, state::HttpAppState};
use crate::app_server::StdioOutput;

pub(super) use proteus_contracts::app_protocol::http::{
    DeleteQueuedMessageRequest, EditQueuedMessageRequest,
};

pub(super) async fn edit(state: &HttpAppState, request: EditQueuedMessageRequest) -> StdioOutput {
    let result = async {
        let server = server_for_session(state, request.session_dir).await?;
        server
            .edit_queued_user_message(request.message_id, request.text)
            .await?;
        Ok(None)
    }
    .await;
    command_response(request.id, result)
}

pub(super) async fn delete(
    state: &HttpAppState,
    request: DeleteQueuedMessageRequest,
) -> StdioOutput {
    let result = async {
        let server = server_for_session(state, request.session_dir).await?;
        server
            .delete_queued_user_message(request.message_id)
            .await?;
        state.emit_session_activity_for_server(&server).await;
        Ok(None)
    }
    .await;
    command_response(request.id, result)
}
