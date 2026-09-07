use futures_util::StreamExt;
use proteus_contracts::{
    contracts::{Model, ProcessModelDescriptor},
    domain::ModelRef,
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, FinishReason, MessageRole,
        ModelCapabilities, ModelStreamEvent, TokenUsage,
    },
};
use proteus_core::core::{
    AppConfig, HeadlessApprovalTransport, ModelExecutionBinding, ModuleCatalog, RuntimeRegistry,
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};

fn request(id: &str) -> CanonicalModelRequest {
    CanonicalModelRequest::new(
        ModelRef::new(id, "fixture"),
        vec![CanonicalMessage::text(MessageRole::User, "hello")],
    )
}

fn response() -> CanonicalModelResponse {
    static RESPONSE: std::sync::OnceLock<CanonicalModelResponse> = std::sync::OnceLock::new();
    RESPONSE
        .get_or_init(|| {
            CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, "hello"),
                vec![],
                FinishReason::Stop,
            )
            .with_provider_metadata(json!({"opaque": {"id": "must-survive"}}))
        })
        .clone()
}

fn settings() -> Value {
    json!({
        "descriptor": ProcessModelDescriptor { adapter_id: "independent-model".into(),
            capabilities: ModelCapabilities::basic_text_and_tools().with_streaming(true), hosted_tools: vec![] },
        "events": [{"TextDelta": {"message_id": response().messages[0].id, "phase": null, "text": "hello"}}],
        "terminal": {"kind": "response", "response": response()}
    })
}

fn config(id: &str, settings: Value) -> AppConfig {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/process_model.py");
    serde_json::from_value(json!({
        "active_provider": "fixture", "providers": {"fixture": {"provider": id, "model": "fixture", "stream": true}},
        "components": {"independent": {"command": "python3", "args": ["-B", fixture],
            "exports": {"model": {id: {"timeout_ms": 10_000}}}}},
        "module_config": {"model": {id: settings}}
    })).unwrap()
}

fn model(config: &AppConfig, cwd: &Path) -> anyhow::Result<Arc<dyn Model>> {
    ModuleCatalog::from_config(config)?.build_model_adapter(&config.active_model_config()?, cwd)
}

#[tokio::test]
async fn arbitrary_model_exports_preserve_exact_canonical_request_stream_and_terminal() {
    let cwd = tempfile::tempdir().unwrap();
    for id in ["python_a", "unrelated_b"] {
        let usage = TokenUsage::new(10, 3);
        let events = vec![
            ModelStreamEvent::TextDelta {
                message_id: response().messages[0].id,
                phase: None,
                text: "hello".into(),
            },
            ModelStreamEvent::ReasoningSummaryDelta {
                text: "summary".into(),
            },
            ModelStreamEvent::Usage { usage },
            ModelStreamEvent::Done {
                finish_reason: FinishReason::Stop,
            },
        ];
        let mut settings = settings();
        settings["events"] = json!(events);
        let input = request(id);
        settings["expected_input"] = json!({"request": input, "stream": true});
        let adapter = model(&config(id, settings), cwd.path()).unwrap();
        assert_eq!(adapter.id(), "independent-model");
        assert!(adapter.capabilities(&request(id).model).supports_streaming);
        let mut stream = adapter.stream(input).await.unwrap();
        let mut actual = Vec::new();
        while let Some(event) = stream.next().await {
            actual.push(event.unwrap());
        }
        let mut expected = events;
        expected.push(ModelStreamEvent::Response {
            response: response(),
        });
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn long_stream_is_not_limited_by_host_work_callback_budget() {
    let cwd = tempfile::tempdir().unwrap();
    let mut settings = settings();
    settings["repeat"] = json!(600); // exceeds the broker's 256 host-work callback budget
    let adapter = model(&config("long", settings), cwd.path()).unwrap();
    let mut stream = adapter.stream(request("long")).await.unwrap();
    let mut count = 0;
    while let Some(event) = stream.next().await {
        if matches!(event.unwrap(), ModelStreamEvent::TextDelta { .. }) {
            count += 1;
        }
    }
    assert_eq!(count, 600);
}

#[tokio::test]
async fn protocol_faults_crashes_and_provider_errors_are_not_success() {
    let cwd = tempfile::tempdir().unwrap();
    for mode in ["bad_sequence", "bad_count", "crash", "forbidden"] {
        let mut settings = settings();
        settings["mode"] = json!(mode);
        let adapter = model(&config(mode, settings), cwd.path()).unwrap();
        assert!(
            adapter.complete(request(mode)).await.is_err(),
            "mode {mode}"
        );
    }
    for kind in ["stream_error", "request_error"] {
        let mut settings = settings();
        settings["terminal"] = json!({"kind": kind, "message": "provider-error"});
        let adapter = model(&config(kind, settings), cwd.path()).unwrap();
        let mut stream = adapter.stream(request(kind)).await.unwrap();
        assert!(matches!(
            stream.next().await.unwrap().unwrap(),
            ModelStreamEvent::TextDelta { .. }
        ));
        let event = stream.next().await.unwrap();
        if kind == "stream_error" {
            assert_eq!(
                event.unwrap(),
                ModelStreamEvent::Error {
                    message: "provider-error".into()
                }
            );
        } else {
            assert_eq!(event.unwrap_err().to_string(), "provider-error");
        }
    }
}

#[tokio::test]
async fn invalid_descriptors_and_response_shapes_fail_at_the_host_boundary() {
    let cwd = tempfile::tempdir().unwrap();
    let mut invalid = settings();
    invalid["descriptor"]["adapter_id"] = json!("");
    assert!(model(&config("bad", invalid), cwd.path()).is_err());
    let mut invalid = settings();
    invalid["descriptor"]["unknown"] = json!(true);
    assert!(model(&config("bad", invalid), cwd.path()).is_err());
    let mut invalid = settings();
    invalid["terminal"]["response"]["messages"] = json!([]);
    let registry =
        RuntimeRegistry::from_config(&config("bad", invalid), cwd.path().to_path_buf()).unwrap();
    let execution = registry.execution_context(
        ModelExecutionBinding::detached(proteus_contracts::contracts::ExecutionScope::fresh(
            Default::default(),
        )),
        Arc::new(HeadlessApprovalTransport),
        proteus_contracts::domain::PermissionMode::Normal,
    );
    assert!(
        execution
            .model
            .complete(request("bad"))
            .await
            .unwrap_err()
            .to_string()
            .contains("model protocol error")
    );
}

async fn marker_is(marker: &Path, expected: &str) {
    marker_is_one_of(marker, &[expected]).await;
}

async fn marker_is_one_of(marker: &Path, expected: &[&str]) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if std::fs::read_to_string(marker)
                .ok()
                .as_deref()
                .is_some_and(|text| expected.contains(&text))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("worker observed cancellation");
}

#[tokio::test]
async fn dropped_unpolled_stream_cancels_without_killing_sibling_export() {
    let cwd = tempfile::tempdir().unwrap();
    let marker = cwd.path().join("cancel");
    let mut pending = settings();
    pending["mode"] = json!("wait");
    pending["marker"] = json!(marker);
    let pid_marker = cwd.path().join("pid");
    pending["pid_marker"] = json!(pid_marker);
    pending["events"] = json!([]);
    let mut cfg = config("pending", pending);
    let mut component = serde_json::to_value(cfg.components.get("independent").unwrap()).unwrap();
    component["exports"]["model"]["sibling"] = json!({});
    cfg.components.insert(
        "independent".into(),
        serde_json::from_value(component).unwrap(),
    );
    cfg.module_config
        .get_mut("model")
        .unwrap()
        .insert("sibling".into(), settings());
    let catalog = ModuleCatalog::from_config(&cfg).unwrap();
    let adapter = catalog
        .build_model_adapter(&cfg.active_model_config().unwrap(), cwd.path())
        .unwrap();
    let mut sibling_config = cfg.active_model_config().unwrap();
    sibling_config.provider = "sibling".into();
    let sibling = catalog
        .build_model_adapter(&sibling_config, cwd.path())
        .unwrap();
    let stream = adapter.stream(request("pending")).await.unwrap();
    marker_is(&marker, "started").await;
    drop(stream);
    marker_is(&marker, "canceled").await;
    assert_eq!(
        sibling.complete(request("sibling")).await.unwrap(),
        response()
    );
    assert_eq!(
        std::fs::read_to_string(pid_marker).unwrap().lines().count(),
        1
    );
}

#[tokio::test]
async fn slow_consumer_is_backpressured_and_drop_releases_callback() {
    let cwd = tempfile::tempdir().unwrap();
    let marker = cwd.path().join("backpressure");
    let mut settings = settings();
    settings["repeat"] = json!(100);
    settings["marker"] = json!(marker);
    let adapter = model(&config("slow", settings), cwd.path()).unwrap();
    let mut stream = adapter.stream(request("slow")).await.unwrap();
    assert!(stream.next().await.unwrap().is_ok());
    tokio::time::sleep(Duration::from_millis(50)).await;
    let count: usize = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
    assert!(
        count <= 2,
        "unbounded streaming: {count} acknowledged events"
    );
    drop(stream);
    // The callback can receive cancellation/consumer closure before the worker
    // reader processes $/cancelRequest. Both paths stop the invocation.
    marker_is_one_of(&marker, &["canceled", "consumer_closed"]).await;
}

#[test]
fn model_requires_explicit_export_and_old_profile_settings_are_rejected() {
    let catalog = ModuleCatalog::new();
    assert!(
        catalog
            .build_model_adapter(&Default::default(), Path::new("."))
            .is_err()
    );
    let error = serde_json::from_value::<AppConfig>(json!({
        "active_provider": "fake", "providers": {"fake": {"provider_config": {}}}
    }))
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unknown field `provider_config`")
    );
}

#[tokio::test]
async fn model_export_deadline_cancels_the_worker_invocation() {
    let cwd = tempfile::tempdir().unwrap();
    let marker = cwd.path().join("timeout");
    let mut pending = settings();
    pending["mode"] = json!("wait");
    pending["events"] = json!([]);
    pending["marker"] = json!(marker);
    let mut cfg = config("timeout", pending);
    let mut component = serde_json::to_value(cfg.components.get("independent").unwrap()).unwrap();
    component["exports"]["model"]["timeout"]["timeout_ms"] = json!(150);
    cfg.components.insert(
        "independent".into(),
        serde_json::from_value(component).unwrap(),
    );
    let adapter = model(&cfg, cwd.path()).unwrap();
    let error = adapter.complete(request("timeout")).await.unwrap_err();
    assert!(format!("{error:#}").contains("timed out"), "{error:#}");
    marker_is(&marker, "canceled").await;
}
