//! Turn admission, ordered event delivery and the existing approval transport.
use super::{
    AppServerHandle, input, internal,
    projection::{self, Projection},
};
use crate::app_server::{AppServerEvent, runs::SendDispatch};
use crate::{
    contracts::{ApprovalCacheScope, CancellationToken, UserInputResponse},
    domain::AgentOutput,
};
use agent_client_protocol::{Client, ConnectionTo, Result, schema::v1::*};
use tokio::sync::{broadcast, oneshot};

pub(super) struct Prompt {
    server: AppServerHandle,
    session: SessionId,
    cancellation: CancellationToken,
    finished: CancellationToken,
    result: oneshot::Receiver<anyhow::Result<AgentOutput>>,
    events: broadcast::Receiver<AppServerEvent>,
    projection: Projection,
    settled: bool,
}

pub(super) async fn prepare(
    server: AppServerHandle,
    request: PromptRequest,
    cancellation: CancellationToken,
) -> Result<Prompt> {
    let text = input::prompt_text(request.prompt)?;
    let events = server.subscribe();
    let SendDispatch::Started(result) = server
        .admit_user_message(None, text, Default::default(), cancellation.clone(), false)
        .await
        .map_err(super::invalid)?
    else {
        return Err(internal("ACP prompt was unexpectedly queued"));
    };
    Ok(Prompt {
        projection: Projection::new(server.session_id()),
        server,
        session: request.session_id,
        cancellation,
        finished: CancellationToken::new(),
        result,
        events,
        settled: false,
    })
}

impl Prompt {
    pub(super) async fn finish(mut self, cx: ConnectionTo<Client>) -> Result<PromptResponse> {
        let result = self.drive(&cx).await;
        self.finished.cancel();
        if result.is_err() {
            self.cancellation.cancel();
            if !self.settled {
                let _ = (&mut self.result).await;
            }
        }
        result
    }

    async fn drive(&mut self, cx: &ConnectionTo<Client>) -> Result<PromptResponse> {
        let result = loop {
            tokio::select! {
                biased;
                event = self.events.recv() => {
                    self.event(event.map_err(internal)?, cx).await?;
                }
                result = &mut self.result => {
                    self.settled = true;
                    break result.map_err(internal)?;
                }
            }
        };
        // Runtime publishes all terminal events before completing the receiver.
        // Drain that same queue before sending the JSON-RPC prompt response.
        loop {
            match self.events.try_recv() {
                Ok(event) => self.event(event, cx).await?,
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(error) => return Err(internal(error)),
            }
        }
        for update in self.projection.settle_tools() {
            self.send(cx, update)?;
        }
        // Runtime deadlines also cancel the token to stop child work. Preserve
        // the typed settlement cause instead of misreporting them as client cancel.
        let output = match result {
            Err(error)
                if crate::core::turn_settlement_status(
                    &error,
                    self.cancellation.is_cancelled(),
                ) == crate::core::TurnSettlementStatus::Canceled =>
            {
                return Ok(PromptResponse::new(StopReason::Cancelled));
            }
            Err(error) => return Err(internal(error)),
            Ok(_) if self.cancellation.is_cancelled() => {
                return Ok(PromptResponse::new(StopReason::Cancelled));
            }
            Ok(output) => output,
        };
        if let Some(update) = self.projection.fallback_output(output.text) {
            self.send(cx, update)?;
        }
        Ok(PromptResponse::new(StopReason::EndTurn))
    }

    fn send(&self, cx: &ConnectionTo<Client>, update: SessionUpdate) -> Result<()> {
        cx.send_notification(SessionNotification::new(self.session.clone(), update))
    }

    async fn event(&mut self, event: AppServerEvent, cx: &ConnectionTo<Client>) -> Result<()> {
        match event {
            AppServerEvent::Runtime { envelope } => {
                for update in self.projection.event(*envelope)? {
                    self.send(cx, update)?;
                }
            }
            AppServerEvent::ApprovalRequested { request } => {
                let request = *request;
                let mut content = vec![ToolCallContent::Content(Content::new(ContentBlock::Text(
                    TextContent::new(request.reason.clone()),
                )))];
                if let Some(preview) = request.preview.as_ref().and_then(|p| p.body.as_ref()) {
                    content.push(ToolCallContent::Content(Content::new(ContentBlock::Text(
                        TextContent::new(preview.clone()),
                    ))));
                }
                let permission = RequestPermissionRequest::new(
                    self.session.clone(),
                    ToolCallUpdate::new(
                        request.call.id.to_string(),
                        ToolCallUpdateFields::new()
                            .title(request.call.name.clone())
                            .status(ToolCallStatus::Pending)
                            .raw_input(request.call.args.clone())
                            .content(content),
                    ),
                    vec![
                        PermissionOption::new(
                            "allow_once",
                            "Allow once",
                            PermissionOptionKind::AllowOnce,
                        ),
                        PermissionOption::new(
                            "reject_once",
                            "Reject",
                            PermissionOptionKind::RejectOnce,
                        ),
                    ],
                );
                let connection = cx.clone();
                let server = self.server.clone();
                let cancellation = self.cancellation.clone();
                let finished = self.finished.clone();
                cx.spawn(async move {
                    let response = tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => None,
                        _ = finished.cancelled() => None,
                        response = connection.send_request(permission).block_task() => response.ok(),
                    };
                    let approved = matches!(response.map(|r| r.outcome), Some(RequestPermissionOutcome::Selected(selected))
                        if selected.option_id.0.as_ref() == "allow_once");
                    // Expired/cancelled prompts may already have removed this responder.
                    let _ = server.respond_approval(&request.approval_id, approved,
                        (!approved).then(|| "Permission rejected or cancelled by ACP client".to_owned()),
                        ApprovalCacheScope::None).await;
                    Ok(())
                })?;
            }
            AppServerEvent::UserInputRequested { request } => {
                // ACP's baseline permission UI cannot express arbitrary question forms.
                // Resolve explicitly instead of leaving the workflow waiting indefinitely.
                let text = format!(
                    "This ACP connection cannot answer structured questions: {}",
                    request
                        .questions
                        .iter()
                        .map(|q| q.question.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                if let Some(update) = projection::message(text) {
                    self.send(cx, update)?;
                }
                let _ = self
                    .server
                    .respond_user_input(&request.request_id, UserInputResponse::empty())
                    .await;
            }
            _ => {}
        }
        Ok(())
    }
}
