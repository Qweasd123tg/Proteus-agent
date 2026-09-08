use super::*;
use proteus_contracts::model_standard::{ModelFailure, ModelFailureKind};

#[test]
fn stream_disconnection_keeps_summary_retry_policy_without_inserting_partial_summary() {
    let partial = CanonicalMessage::text(MessageRole::Assistant, "incomplete summary");
    let failure = ModelFailure::new(ModelFailureKind::StreamDisconnected, "stream disconnected")
        .with_completed_messages(vec![partial.clone()]);
    let mut host = TestHost::with_results(vec![
        Err(ProcessModuleError::from_model_failure(failure)),
        Ok(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "complete summary"),
            Vec::new(),
            FinishReason::Stop,
        )),
    ]);
    let output = compact_with_host(
        input(vec![CanonicalMessage::text(MessageRole::User, "work")], 500),
        &mut host,
    );
    assert!(output.changed);
    assert_eq!(
        output.summary.as_deref(),
        Some(format!("{SUMMARY_PREFIX}\ncomplete summary").as_str())
    );
    let requests = host.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| !request.messages.contains(&partial))
    );
    assert_eq!(requests[0].messages.len(), requests[1].messages.len());
}
