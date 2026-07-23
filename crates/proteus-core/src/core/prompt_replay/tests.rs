use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use async_trait::async_trait;
use futures_util::stream;
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::{
    contracts::{Model, ModelEventStream},
    core::{JournalEntry, ModelRequestRecorded, ModelResponseRecorded, SessionStore, TurnOpened},
    domain::{
        AgentTask, CacheHints, Citation, HostedToolActivity, HostedToolConfig, HostedToolStatus,
        ModelRef, ReasoningConfig, ToolCall, ToolSafety, ToolSpec, ToolSurface, WebSearchAction,
        WebSearchHostedToolConfig, new_exchange_id, new_session_id, new_thread_id, new_turn_id,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        InstructionBlock, InstructionKind, MessageRole, ModelCapabilities, ModelStreamEvent,
        TokenUsage,
    },
};

#[derive(Clone)]
struct RecordingModel {
    requests: Arc<Mutex<Vec<CanonicalModelRequest>>>,
    response: CanonicalModelResponse,
}

impl RecordingModel {
    fn new(response: CanonicalModelResponse) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            response,
        }
    }

    fn requests(&self) -> Vec<CanonicalModelRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

#[async_trait]
impl Model for RecordingModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "test.recording".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
            .with_tools(true)
            .with_streaming(true)
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        self.requests.lock().expect("requests lock").push(request);
        let response = self.response.clone();
        Ok(Box::pin(stream::once(async move {
            Ok(ModelStreamEvent::Response { response })
        })))
    }
}

struct TestJournal {
    _config_dir: TempDir,
    _workspace: TempDir,
    store: SessionStore,
    thread_id: ThreadId,
    turn_id: TurnId,
}

impl TestJournal {
    async fn new() -> Self {
        let config_dir = tempfile::tempdir().expect("config dir");
        let workspace = tempfile::tempdir().expect("workspace");
        let store = SessionStore::new(config_dir.path(), workspace.path(), new_session_id())
            .expect("session store");
        let thread_id = new_thread_id();
        let turn_id = new_turn_id();
        store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::TurnOpened(TurnOpened {
                    task: AgentTask::new("replay fixture", workspace.path().to_path_buf()),
                    base_history_revision: 0,
                    module_epoch: 0,
                    config_snapshot: json!({}),
                }),
            )
            .await
            .expect("turn opened");
        Self {
            _config_dir: config_dir,
            _workspace: workspace,
            store,
            thread_id,
            turn_id,
        }
    }

    async fn append_request(&self, exchange_id: ExchangeId, request: CanonicalModelRequest) {
        self.store
            .append_journal_entry(
                self.thread_id,
                Some(self.turn_id),
                JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                    exchange_id,
                    request,
                }),
            )
            .await
            .expect("model request");
    }

    async fn append_response(&self, exchange_id: ExchangeId, outcome: ModelResponseOutcome) {
        self.store
            .append_journal_entry(
                self.thread_id,
                Some(self.turn_id),
                JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                    exchange_id,
                    outcome,
                }),
            )
            .await
            .expect("model response");
    }

    async fn append_complete_exchange(
        &self,
        request: CanonicalModelRequest,
        response: CanonicalModelResponse,
    ) -> ExchangeId {
        let exchange_id = new_exchange_id();
        self.append_request(exchange_id, request).await;
        self.append_response(exchange_id, ModelResponseOutcome::Response { response })
            .await;
        exchange_id
    }
}

fn request(text: &str) -> CanonicalModelRequest {
    CanonicalModelRequest::new(
        ModelRef::new("fake", "recorded-model"),
        vec![CanonicalMessage::text(MessageRole::User, text)],
    )
}

fn text_response(text: &str) -> CanonicalModelResponse {
    CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, text),
        Vec::new(),
        FinishReason::Stop,
    )
}

#[tokio::test]
async fn single_complete_exchange_is_selected_without_id() {
    let journal = TestJournal::new().await;
    let exchange_id = journal
        .append_complete_exchange(request("saved"), text_response("recorded"))
        .await;
    let model = RecordingModel::new(text_response("replayed"));

    let report = replay_prompt(
        journal.store.session_dir(),
        Arc::new(model.clone()),
        PromptReplayOptions::default(),
    )
    .await
    .expect("prompt replay");

    assert_eq!(report.source.exchange_id, exchange_id);
    assert_eq!(report.source.session_id, journal.store.session_id());
    assert_eq!(model.requests().len(), 1);
}

#[tokio::test]
async fn multiple_exchanges_require_id_and_list_available_ids() {
    let journal = TestJournal::new().await;
    let first = journal
        .append_complete_exchange(request("first"), text_response("one"))
        .await;
    let second_request = request("second");
    let second = journal
        .append_complete_exchange(second_request.clone(), text_response("two"))
        .await;
    let model = RecordingModel::new(text_response("unused"));

    let error = replay_prompt(
        journal.store.session_dir(),
        Arc::new(model.clone()),
        PromptReplayOptions::default(),
    )
    .await
    .expect_err("ambiguous replay must fail");

    let message = error.to_string();
    assert!(message.contains("multiple model exchanges"));
    assert!(message.contains(&first.to_string()));
    assert!(message.contains(&second.to_string()));
    assert!(model.requests().is_empty());

    let report = replay_prompt(
        journal.store.session_dir(),
        Arc::new(model.clone()),
        PromptReplayOptions {
            exchange_id: Some(second),
            allow_hosted_tools: false,
        },
    )
    .await
    .expect("explicit exchange replay");
    assert_eq!(report.source.exchange_id, second);
    assert_eq!(model.requests(), vec![second_request]);
}

#[tokio::test]
async fn unknown_and_incomplete_exchanges_are_rejected() {
    let complete = TestJournal::new().await;
    let available = complete
        .append_complete_exchange(request("known"), text_response("recorded"))
        .await;
    let model = RecordingModel::new(text_response("unused"));
    let unknown = new_exchange_id();
    let error = replay_prompt(
        complete.store.session_dir(),
        Arc::new(model.clone()),
        PromptReplayOptions {
            exchange_id: Some(unknown),
            allow_hosted_tools: false,
        },
    )
    .await
    .expect_err("unknown exchange must fail");
    assert!(error.to_string().contains(&unknown.to_string()));
    assert!(error.to_string().contains(&available.to_string()));

    let incomplete = TestJournal::new().await;
    let incomplete_id = new_exchange_id();
    incomplete
        .append_request(incomplete_id, request("interrupted"))
        .await;
    for exchange_id in [None, Some(incomplete_id)] {
        let error = replay_prompt(
            incomplete.store.session_dir(),
            Arc::new(model.clone()),
            PromptReplayOptions {
                exchange_id,
                allow_hosted_tools: false,
            },
        )
        .await
        .expect_err("incomplete exchange must fail");
        assert!(error.to_string().contains("is incomplete"));
        assert!(error.to_string().contains(&incomplete_id.to_string()));
    }
    assert!(model.requests().is_empty());
}

#[tokio::test]
async fn exact_saved_request_is_forwarded_and_journal_is_unchanged() {
    let journal = TestJournal::new().await;
    let mut client_metadata = BTreeMap::new();
    client_metadata.insert("trace".to_owned(), "preserve-me".to_owned());
    let saved_request = request("exact input")
        .with_instructions(vec![InstructionBlock::new(
            InstructionKind::Developer,
            "exact instruction",
            90,
        )])
        .with_cache(CacheHints::new(true, true).with_routing_key("saved-routing-key"))
        .with_reasoning(ReasoningConfig::new(Some("high".to_owned()), true))
        .with_client_metadata(client_metadata)
        .with_metadata(json!({ "after_shaper": true }));
    let recorded_response = text_response("recorded").with_usage(TokenUsage::new(10, 2));
    journal
        .append_complete_exchange(saved_request.clone(), recorded_response)
        .await;
    let model = RecordingModel::new(text_response("replayed").with_usage(TokenUsage::new(11, 3)));
    let before = std::fs::read(journal.store.journal_path()).expect("journal before");

    let report = replay_prompt(
        journal.store.journal_path(),
        Arc::new(model.clone()),
        PromptReplayOptions::default(),
    )
    .await
    .expect("prompt replay");

    let after = std::fs::read(journal.store.journal_path()).expect("journal after");
    assert_eq!(model.requests(), vec![saved_request]);
    assert_eq!(before, after, "prompt replay modified the source journal");
    assert_eq!(
        report.recorded_model,
        ModelRef::new("fake", "recorded-model")
    );
    assert_eq!(report.replay_model, report.recorded_model);
    assert_eq!(report.usage.recorded, Some(TokenUsage::new(10, 2)));
    assert_eq!(report.usage.replay, Some(TokenUsage::new(11, 3)));
    assert_eq!(report.text_equal, Some(false));

    let json = serde_json::to_value(&report).expect("serialize report");
    assert_eq!(json["schema_version"], 1);
    assert_eq!(
        json["source"]["exchange_id"],
        report.source.exchange_id.to_string()
    );
    assert_eq!(json["recorded_outcome"]["status"], "response");
    assert_eq!(json["replay_outcome"]["status"], "response");
    assert_eq!(json["local_tool_calls"]["replay"], 0);
}

#[tokio::test]
async fn replay_reports_local_tool_call_without_executing_it() {
    let journal = TestJournal::new().await;
    let marker = journal._workspace.path().join("must-not-exist");
    let saved_request = request("ask for a tool").with_tools(vec![ToolSpec::new(
        "write_file",
        "write a file",
        json!({ "type": "object" }),
        ToolSafety::WritesFiles,
    )]);
    journal
        .append_complete_exchange(saved_request, text_response("recorded"))
        .await;
    let call = ToolCall::new(
        "call-local",
        "write_file",
        json!({ "path": marker, "content": "should not be written" }),
    );
    let replay_response = CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: call.clone() }],
        ),
        vec![call],
        FinishReason::ToolCalls,
    );

    let report = replay_prompt(
        journal.store.session_dir(),
        Arc::new(RecordingModel::new(replay_response)),
        PromptReplayOptions::default(),
    )
    .await
    .expect("prompt replay");

    assert_eq!(report.local_tool_calls.replay, 1);
    assert_eq!(report.local_tool_call_names.replay, vec!["write_file"]);
    assert!(!marker.exists(), "replay executed a local tool call");
}

#[tokio::test]
async fn provider_hosted_tools_require_explicit_opt_in_and_request_stays_exact() {
    let journal = TestJournal::new().await;
    let hosted = ToolSpec::new(
        "web_search",
        "search the web",
        json!({}),
        ToolSafety::Network,
    )
    .with_surface(ToolSurface::provider_hosted(HostedToolConfig::WebSearch {
        config: WebSearchHostedToolConfig::default(),
    }));
    let saved_request = request("hosted search").with_tools(vec![hosted]);
    let hosted_response = CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![
                ContentPart::Text {
                    text: "result".to_owned(),
                },
                ContentPart::HostedToolActivity {
                    activity: HostedToolActivity::WebSearch {
                        id: "search-1".to_owned(),
                        status: HostedToolStatus::Completed,
                        action: WebSearchAction::Search {
                            queries: vec!["proteus".to_owned()],
                            sources: Vec::new(),
                        },
                    },
                },
                ContentPart::Citation {
                    citation: Citation::Url {
                        start_index: 0,
                        end_index: 6,
                        title: "source".to_owned(),
                        url: "https://example.invalid".to_owned(),
                    },
                },
            ],
        ),
        Vec::new(),
        FinishReason::Stop,
    );
    journal
        .append_complete_exchange(saved_request.clone(), hosted_response.clone())
        .await;
    let model = RecordingModel::new(hosted_response);

    let error = replay_prompt(
        journal.store.session_dir(),
        Arc::new(model.clone()),
        PromptReplayOptions::default(),
    )
    .await
    .expect_err("hosted replay must fail closed");
    assert!(
        error
            .to_string()
            .contains("provider-hosted tools [web_search]")
    );
    assert!(error.to_string().contains("--allow-hosted-tools"));
    assert!(model.requests().is_empty());

    let report = replay_prompt(
        journal.store.session_dir(),
        Arc::new(model.clone()),
        PromptReplayOptions {
            exchange_id: None,
            allow_hosted_tools: true,
        },
    )
    .await
    .expect("explicit hosted replay");

    assert_eq!(model.requests(), vec![saved_request]);
    assert_eq!(report.request_hosted_tools, vec!["web_search"]);
    assert!(report.hosted_tools_allowed);
    assert_eq!(report.hosted_activities.recorded, 1);
    assert_eq!(report.hosted_activities.replay, 1);
    assert_eq!(report.citations.recorded, 1);
    assert_eq!(report.citations.replay, 1);
}
