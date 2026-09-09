use super::*;
use proteus_contracts::model_standard::{ModelFailure, ModelFailureKind};

fn failure(
    kind: ModelFailureKind,
    message: &str,
) -> Result<CanonicalModelResponse, ProcessModuleError> {
    Err(ProcessModuleError::from_model_failure(ModelFailure::new(
        kind, message,
    )))
}

fn recovery_input(retries: u64) -> CompactionInput {
    input(vec![CanonicalMessage::text(MessageRole::User, "work")], 500)
        .with_config(json!({"trigger_tokens": 100, "stream_max_retries": retries}))
}

#[test]
fn retry_budget_stops_on_the_exact_last_failure_including_zero_retries() {
    for retries in [0, 1, 2] {
        let mut host = TestHost::with_results(
            (0..=retries)
                .map(|attempt| failure(ModelFailureKind::Other, &format!("failure {attempt}")))
                .collect(),
        );
        let error = compact(recovery_input(retries), &mut host).unwrap_err();
        assert_eq!(
            error.model_failure.unwrap().message,
            format!("failure {retries}")
        );
        let requests = host.requests.lock().unwrap();
        assert_eq!(requests.len(), retries as usize + 1);
        let texts: Vec<_> = requests
            .iter()
            .map(|request| {
                request
                    .messages
                    .iter()
                    .map(message_text)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(texts.iter().all(|text| text == &texts[0]));
    }
}

#[test]
fn trimming_resets_retry_budget_and_prompt_only_overflow_is_terminal() {
    let mut host = TestHost::with_results(vec![
        failure(ModelFailureKind::Other, "first failure"),
        failure(ModelFailureKind::ContextWindowExceeded, "remove work"),
        failure(ModelFailureKind::Other, "budget was reset"),
        failure(
            ModelFailureKind::ContextWindowExceeded,
            "prompt alone does not fit",
        ),
    ]);
    let error = compact(recovery_input(1), &mut host).unwrap_err();
    let failure = error.model_failure.unwrap();
    assert_eq!(failure.kind, ModelFailureKind::ContextWindowExceeded);
    assert_eq!(failure.message, "prompt alone does not fit");
    let requests = host.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.messages.len())
            .collect::<Vec<_>>(),
        [2, 2, 1, 1]
    );
    assert_eq!(requests[3].messages[0].display_text(), COMPACTION_PROMPT);
}

#[test]
fn session_budget_and_cancel_do_not_consume_summary_retries() {
    for kind in [
        ModelFailureKind::SessionBudgetExceeded,
        ModelFailureKind::Interrupted,
    ] {
        let mut host = TestHost::with_results(vec![failure(kind, "terminal")]);
        let error = compact(recovery_input(100), &mut host).unwrap_err();
        assert_eq!(error.model_failure.unwrap().kind, kind);
        assert_eq!(host.requests.lock().unwrap().len(), 1);
    }
    let mut host = TestHost {
        cancelled: true,
        ..TestHost::default()
    };
    assert!(compact(recovery_input(100), &mut host).is_err());
    assert!(host.requests.lock().unwrap().is_empty());
}

#[test]
fn retry_config_matches_provider_bounds_and_rejects_invalid_forms() {
    use crate::config::CompactorConfig;
    assert_eq!(
        CompactorConfig::parse(&json!({}))
            .unwrap()
            .stream_max_retries,
        5
    );
    assert_eq!(
        CompactorConfig::parse(&json!({"stream_max_retries": 999}))
            .unwrap()
            .stream_max_retries,
        100
    );
    for value in [json!(-1), json!(1.5), json!("2"), json!(null)] {
        assert!(CompactorConfig::parse(&json!({"stream_max_retries": value})).is_err());
    }
}

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
