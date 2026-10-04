use super::*;
use proteus_contracts::{
    contracts::ProcessModelDescriptor,
    domain::{ImageAttachment, UserMessageInput, new_session_id, new_thread_id},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities,
    },
};
use proteus_core::core::{
    ModuleCatalog, TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
};

const PNG: &[u8] = include_bytes!("../fixtures/pixel.png");

fn config(images_supported: bool, capture: &Path) -> AppConfig {
    let response = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "fixture"),
        vec![],
        FinishReason::Stop,
    );
    serde_json::from_value(json!({
        "active_provider":"vision", "providers":{"vision":{"provider":"vision","model":"fixture","stream":true}},
        "modules":{"workflow":"coding.single_loop", "policy":"allow_all"},
        "components":{
            "workflow":{"command":test_model::reference_module(),"exports":{"workflow":{"coding.single_loop":{}}, "policy":{"allow_all":{}}}},
            "vision":{"command":"python3","args":["-B",workspace_file("crates/proteus-core/tests/fixtures/process_model.py")],"exports":{"model":{"vision":{}}}}
        },
        "module_config":{"model":{"vision":{
            "descriptor":ProcessModelDescriptor {adapter_id:"vision-probe".into(), capabilities:ModelCapabilities::basic_text_and_tools().with_streaming(true).with_image_input(images_supported), hosted_tools:vec![]},
            "mode":"image_probe", "capture_path":capture, "terminal":{"kind":"response","response":response}
        }}}
    })).unwrap()
}

fn input(text: &str) -> UserMessageInput {
    UserMessageInput {
        text: text.into(),
        images: vec![ImageAttachment::from_bytes("board.png".into(), PNG).unwrap()],
    }
}

#[tokio::test]
async fn image_input_survives_process_workflow_cold_resume_and_replay() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = workspace.path().join("requests.jsonl");
    let config = config(true, &capture);
    let thread = new_thread_id();
    let config_path = workspace.path().join("config.toml");
    let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
        .with_config_path(Some(&config_path))
        .with_session_ids(new_session_id(), thread)
        .build_async()
        .await
        .unwrap();
    let mut invalid = input("Invalid MIME");
    invalid.images[0].mime_type = "image/jpeg".into();
    let error = runtime
        .run_input(invalid, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("invalid image format or MIME type"));
    assert!(
        runtime.history().await.is_empty(),
        "invalid attachment must fail before opening a turn"
    );
    assert!(!capture.exists());
    assert_eq!(
        runtime
            .run_input(input("Describe this board"), CancellationToken::new())
            .await
            .unwrap()
            .text,
        "Images received: 1"
    );
    let history = runtime.history().await;
    let image = history[0]
        .parts
        .iter()
        .find_map(|part| match &part.payload {
            ContentPart::Image { image } => Some(image.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(std::fs::read(&image.path).unwrap(), PNG);
    assert_eq!(runtime.image_bytes(&image.id).unwrap().0, PNG);
    let session_dir = runtime.session_dir().unwrap().to_path_buf();
    drop(runtime);
    let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
        .resume_from_session_dir(&session_dir, thread)
        .unwrap()
        .build_async()
        .await
        .unwrap();
    assert_eq!(
        runtime
            .run("What else do you see?".into())
            .await
            .unwrap()
            .text,
        "Images received: 1"
    );
    assert!(runtime.history().await[0].parts.iter().any(
        |part| matches!(&part.payload, ContentPart::Image { image: restored } if restored == &image)
    ));
    let turn_id = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_records()
        .unwrap()
        .iter()
        .rev()
        .find_map(|record| match &record.entry {
            JournalEntry::TurnSettled(_) => record.turn_id,
            _ => None,
        })
        .unwrap();
    let replay = replay_workflow(
        &session_dir,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions {
            turn_id: Some(turn_id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
    assert_eq!(
        std::fs::read_to_string(capture).unwrap().lines().count(),
        2,
        "replay must not invoke the provider"
    );
}

#[tokio::test]
async fn text_only_model_rejects_image_before_provider_invocation() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = workspace.path().join("requests.jsonl");
    let config = config(false, &capture);
    let config_path = workspace.path().join("config.toml");
    let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
        .with_config_path(Some(&config_path))
        .build_async()
        .await
        .unwrap();
    let error = runtime
        .run_input(input("Describe"), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("does not support image input"));
    assert!(!capture.exists());
    let session_dir = runtime.session_dir().unwrap().to_owned();
    drop(runtime);
    let cold = SessionStore::open(session_dir).unwrap();
    assert!(cold.load_records().unwrap().iter().any(|record| matches!(&record.entry, JournalEntry::TurnSettled(settlement) if settlement.status == TurnSettlementStatus::Error)));
    assert!(
        cold.load_messages().unwrap()[0]
            .parts
            .iter()
            .any(|part| matches!(part.payload, ContentPart::Image { .. }))
    );
    assert!(!capture.exists());
}

#[tokio::test]
async fn codex_compaction_strips_working_images_but_keeps_original_journal_and_store() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = workspace.path().join("compaction-requests.jsonl");
    let mut value = serde_json::to_value(config(true, &capture)).unwrap();
    value["modules"]["workflow"] = json!("coding.codex_loop");
    value["modules"]["compactor"] = json!("codex");
    value["components"]["workflow"]["exports"]["workflow"] = json!({"coding.codex_loop":{}});
    value["components"]["workflow"]["exports"]["compactor"] = json!({"codex":{}});
    value["module_config"]["compactor"] = json!({"codex":{"trigger_tokens":1}});
    let config: AppConfig = serde_json::from_value(value).unwrap();
    let thread = new_thread_id();
    let config_path = workspace.path().join("config.toml");
    let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
        .with_config_path(Some(&config_path))
        .with_session_ids(new_session_id(), thread)
        .build_async()
        .await
        .unwrap();
    assert_eq!(
        runtime
            .run_input(input(""), CancellationToken::new())
            .await
            .unwrap()
            .text,
        "Images received: 1"
    );
    let original = runtime.history().await[0].clone();
    let image = original
        .parts
        .iter()
        .find_map(|part| match &part.payload {
            ContentPart::Image { image } => Some(image.clone()),
            _ => None,
        })
        .unwrap();
    // Incoming image is excluded from pre-turn compaction and still reaches the
    // first ordinary model call, while the historical image is summarized.
    assert_eq!(
        runtime
            .run_input(input("New board"), CancellationToken::new())
            .await
            .unwrap()
            .text,
        "Images received: 1"
    );
    assert_eq!(
        runtime.run("Text follow-up".into()).await.unwrap().text,
        "Images received: 0"
    );
    let working = runtime.history().await;
    assert!(
        working
            .iter()
            .flat_map(|message| &message.parts)
            .all(|part| !matches!(part.payload, ContentPart::Image { .. }))
    );
    assert_eq!(runtime.image_bytes(&image.id).unwrap().0, PNG);
    let session_dir = runtime.session_dir().unwrap().to_path_buf();
    drop(runtime);
    let cold = SessionStore::open(session_dir.clone()).unwrap();
    assert_eq!(cold.load_messages().unwrap(), working);
    let records = cold.load_records().unwrap();
    assert!(records.iter().any(|record| matches!(&record.entry,
        JournalEntry::HistoryMutated(mutation) if mutation.messages.iter().any(|message| message == &original))));
    assert_eq!(std::fs::read(&image.path).unwrap(), PNG);
    let turn_id = records
        .iter()
        .rev()
        .find_map(|record| match &record.entry {
            JournalEntry::TurnSettled(_) => record.turn_id,
            _ => None,
        })
        .unwrap();
    let replay = replay_workflow(
        &session_dir,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions {
            turn_id: Some(turn_id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
    let requests = std::fs::read_to_string(capture)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<CanonicalModelRequest>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 5, "replay must not invoke the provider");
    let image_counts = requests
        .iter()
        .map(|request| {
            request
                .messages
                .iter()
                .flat_map(|message| &message.parts)
                .filter(|part| matches!(part.payload, ContentPart::Image { .. }))
                .count()
        })
        .collect::<Vec<_>>();
    assert_eq!(image_counts, [1, 1, 1, 1, 0]);
}
