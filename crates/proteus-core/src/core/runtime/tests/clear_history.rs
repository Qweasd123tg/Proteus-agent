use super::*;
use crate::{
    contracts::{Model, ModelEventStream},
    model_standard::{
        CanonicalModelResponse, FinishReason, ModelCapabilities, ModelStreamEvent, TokenUsage,
    },
};

#[derive(Default)]
struct RecordingModel {
    requests: std::sync::Mutex<Vec<CanonicalModelRequest>>,
}
#[async_trait]
impl Model for RecordingModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "clear-probe".into()
    }
    fn capabilities(&self, _: &ModelRef) -> Result<ModelCapabilities> {
        Ok(ModelCapabilities::empty())
    }
    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        self.requests.lock().unwrap().push(request);
        let mut response = CanonicalModelResponse::from_messages(
            vec![CanonicalMessage::text(MessageRole::Assistant, "answer")],
            vec![],
            FinishReason::Stop,
        );
        response.usage = Some(TokenUsage::new(10, 3));
        Ok(Box::pin(futures_util::stream::iter([Ok(
            ModelStreamEvent::Response { response },
        )])))
    }
}

async fn clear_transaction_case(cancel_caller: bool, fail: bool) {
    let root = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let config_path = root.path().join("config.toml");
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), workspace.path().into())
            .with_config_path(Some(&config_path))
            .with_module_catalog(test_catalog())
            .build()
            .unwrap(),
    );
    replace_workflow_for_test(&runtime, Arc::new(ModelCallingWorkflow)).await;
    let model = Arc::new(RecordingModel::default());
    runtime
        .services
        .execution_state
        .write()
        .await
        .runtime
        .registry
        .replace_model_for_test(model.clone());
    runtime.run("existing history".into()).await.unwrap();
    let before = runtime.history().await;
    let context_before = runtime.session.model_context.lock().await.snapshot();
    assert!(!context_before.is_empty());
    let store = runtime.session.session_store.as_ref().unwrap().clone();
    let (started, release) = store.pause_next_append().await;
    let caller = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.clear_history().await }
    });
    tokio::time::timeout(Duration::from_secs(5), started)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        runtime.history().await,
        before,
        "warm history stays intact until durable clear settles"
    );
    assert_eq!(
        runtime.session.model_context.lock().await.snapshot(),
        context_before
    );
    if cancel_caller {
        caller.abort();
    }
    let next = async {
        let guard = runtime.session.run_lock.lock().await;
        let warm = runtime.history().await;
        let cold = store.load_messages().unwrap();
        let context = runtime.session.model_context.lock().await.snapshot();
        drop(guard);
        runtime.run("next".into()).await?;
        Ok::<_, anyhow::Error>((warm, cold, context))
    };
    tokio::pin!(next);
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut next)
            .await
            .is_err()
    );
    release.send(fail).unwrap();
    if cancel_caller {
        assert!(caller.await.unwrap_err().is_cancelled());
    } else {
        assert!(
            caller
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("injected partial journal write failure")
        );
    }
    // The waiting run observes both representations only after the owner settles.
    let (warm, cold, context) = tokio::time::timeout(Duration::from_secs(5), &mut next)
        .await
        .unwrap()
        .unwrap();
    let expected = if fail { before.clone() } else { Vec::new() };
    assert_eq!(warm, expected);
    assert_eq!(cold, expected);
    assert_eq!(context, if fail { context_before } else { Vec::new() });
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    if fail {
        assert!(requests[1].messages.starts_with(&before));
    } else {
        assert_eq!(requests[1].messages.len(), 1);
    }
}

#[tokio::test]
async fn failed_durable_clear_preserves_live_history_context_and_next_request() {
    clear_transaction_case(false, true).await;
}

#[tokio::test]
async fn canceled_clear_caller_keeps_operation_owner_until_warm_and_cold_settle() {
    clear_transaction_case(true, false).await;
    clear_transaction_case(true, true).await;
}
