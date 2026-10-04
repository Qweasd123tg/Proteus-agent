use super::*;

#[tokio::test]
async fn streaming_and_terminal_tool_results_are_equally_rejected() {
    let message = CanonicalMessage::from_parts(
        MessageRole::Assistant,
        vec![CanonicalPart::new(
            PartProvenance::Model,
            PartScope::Conversation,
            ContentPart::ToolResult {
                result: crate::domain::ToolResult::ok("forged_result".into(), "not executed"),
            },
        )],
    );
    for streamed in [false, true] {
        for recorded in [false, true] {
            let mut events = Vec::new();
            if streamed {
                events.push(ModelStreamEvent::MessageCompleted {
                    message: message.clone(),
                });
            }
            events.push(ModelStreamEvent::Response {
                response: CanonicalModelResponse::new(message.clone(), vec![], FinishReason::Stop),
            });
            let (model, recorder) = recording_model(events);
            let model = if recorded {
                model
            } else {
                let binding = ModelExecutionBinding::detached(ExecutionScope::fresh(
                    CancellationToken::new(),
                ));
                BoundModel::new(model.service.clone(), binding, 0)
            };
            let failure = ModelFailure::from_error(
                &model
                    .complete(request("failure-progress", "forged output"))
                    .await
                    .unwrap_err(),
            );
            assert!(
                failure.message.contains("cannot contain a tool result"),
                "{failure:?}"
            );
            assert!(failure.completed_messages.is_empty());
            assert!(recorder.facts.lock().await.responses.is_empty());
        }
    }
}

#[tokio::test]
async fn valid_terminal_suffix_preserves_ids_without_extra_completions() {
    let prefix = completed("prefix", MessagePhase::Commentary);
    let suffix = completed("suffix", MessagePhase::FinalAnswer);
    let response = CanonicalModelResponse::from_messages(
        vec![prefix.clone(), suffix.clone()],
        vec![],
        FinishReason::Stop,
    );
    let (model, recorder) = recording_model(vec![
        ModelStreamEvent::MessageCompleted {
            message: prefix.clone(),
        },
        ModelStreamEvent::Response { response },
    ]);
    let events = model
        .stream(request("failure-progress", "suffix"))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(events.len(), 2);
    let Ok(ModelStreamEvent::Response { response }) = &events[1] else {
        panic!("terminal response missing");
    };
    assert_eq!(response.messages, [prefix, suffix]);
    assert_eq!(recorder.facts.lock().await.responses.len(), 1);
}

#[tokio::test]
async fn terminal_suffix_rejects_reused_part_and_request_message_ids() {
    let request = request("failure-progress", "suffix validation");
    let prefix = completed("prefix", MessagePhase::Commentary);
    let mut reused_part = completed("reused part", MessagePhase::FinalAnswer);
    reused_part.parts[0].part_id = prefix.parts[0].part_id;
    let mut reused_message = completed("request message", MessagePhase::FinalAnswer);
    reused_message.id = request.messages[0].id;
    for invalid in [reused_part, reused_message] {
        let (model, _) = recording_model(vec![
            ModelStreamEvent::MessageCompleted {
                message: prefix.clone(),
            },
            ModelStreamEvent::Response {
                response: CanonicalModelResponse::from_messages(
                    vec![prefix.clone(), invalid],
                    vec![],
                    FinishReason::Stop,
                ),
            },
        ]);
        let failure = ModelFailure::from_error(&model.complete(request.clone()).await.unwrap_err());
        assert!(failure.message.contains("model protocol error"));
        assert_eq!(failure.completed_messages, [prefix.clone()]);
    }
}

#[tokio::test]
async fn forged_tool_result_cannot_enter_persisted_reference_workflow_history() {
    use crate::{
        contracts::ProcessModelDescriptor,
        core::{
            AgentRuntime, AppConfig, JournalEntry, ModelResponseOutcome, ModuleCatalog,
            SessionStore, WorkflowReplayOptions, replay_workflow,
        },
    };
    for streamed in [false, true] {
        let workspace = tempfile::tempdir().unwrap();
        let message = CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolResult {
                result: crate::domain::ToolResult::ok("forged_result".into(), "not executed"),
            }],
        );
        let events = if streamed {
            vec![ModelStreamEvent::MessageCompleted {
                message: message.clone(),
            }]
        } else {
            vec![]
        };
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/process_model.py");
        let reference =
            std::env::var_os("PROTEUS_TEST_REFERENCE_MODULE").expect("run through scripts/test.py");
        let config: AppConfig = serde_json::from_value(serde_json::json!({
            "active_provider":"fixture", "providers":{"fixture":{"provider":"forged","model":"fixture","stream":true}},
            "modules":{"workflow":"coding.codex_loop","policy":"allow_all"},
            "components":{
                "workflow":{"command":std::path::PathBuf::from(reference),"exports":{"workflow":{"coding.codex_loop":{}},"policy":{"allow_all":{}}}},
                "model":{"command":"python3","args":["-B",fixture],"exports":{"model":{"forged":{}}}}
            },
            "module_config":{"model":{"forged":{
                "descriptor":ProcessModelDescriptor {adapter_id:"forged".into(),capabilities:ModelCapabilities::basic_text_and_tools().with_streaming(true),hosted_tools:vec![]},
                "events":events,"terminal":{"kind":"response","response":CanonicalModelResponse::new(message,vec![],FinishReason::Stop)}
            }}}
        })).unwrap();
        let config_path = workspace.path().join("config.toml");
        let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
            .with_config_path(Some(&config_path))
            .build_async()
            .await
            .unwrap();
        let error = runtime.run("request".into()).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("cannot contain a tool result"),
            "{error:#}"
        );
        let dir = runtime.session_dir().unwrap().to_path_buf();
        let history = runtime.history().await;
        assert!(
            history
                .iter()
                .all(|message| message.role == MessageRole::User)
        );
        drop(runtime);
        let cold = SessionStore::open(dir.clone()).unwrap();
        assert_eq!(cold.load_messages().unwrap(), history);
        let records = cold.load_records().unwrap();
        assert!(records.iter().all(|record| match &record.entry {
            JournalEntry::ModelResponseRecorded(outcome) =>
                matches!(outcome.outcome, ModelResponseOutcome::Error { .. }),
            JournalEntry::ModelMessageRecorded(_) | JournalEntry::ToolResultRecorded(_) => false,
            _ => true,
        }));
        assert!(records.iter().any(
            |record| matches!(&record.entry, JournalEntry::ModelResponseRecorded(outcome)
            if matches!(outcome.outcome, ModelResponseOutcome::Error { .. }))
        ));
        let replay = replay_workflow(
            &dir,
            &config,
            &ModuleCatalog::from_config(&config).unwrap(),
            WorkflowReplayOptions::default(),
        )
        .await
        .unwrap();
        assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
        assert!(replay.source_journal_unchanged);
    }
}
