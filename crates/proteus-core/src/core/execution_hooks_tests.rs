use super::*;
use crate::contracts::HookResponse;
use crate::{
    contracts::{Model, ModelCallOrigin, ModelEventStream, NoopExecutionRecorder},
    domain::{ModelRef, ToolCall, new_call_id, new_execution_id},
    model_standard::{CanonicalModelRequest, InstructionBlock, InstructionKind, ModelCapabilities},
};
use std::sync::Mutex;
struct MockModel;
#[async_trait]
impl Model for MockModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "mock".into()
    }
    fn capabilities(&self, _: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }
    async fn stream(&self, _: CanonicalModelRequest) -> Result<ModelEventStream> {
        panic!("hooks must not invoke provider")
    }
}
struct Handler {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
    fail: bool,
    block: bool,
}
#[async_trait]
impl HookHandler for Handler {
    async fn invoke(
        &self,
        input: HookInput,
        cancellation: CancellationToken,
    ) -> Result<HookResponse> {
        assert!(!cancellation.is_cancelled());
        self.calls.lock().unwrap().push(self.name.into());
        if self.fail {
            anyhow::bail!("handler failed")
        }
        if self.block {
            return Ok(HookResponse::BlockTool {
                reason: "owner block".into(),
            });
        }
        match input.event {
            HookEvent::BeforeModel { request, .. } => {
                let mut instructions = request.instructions;
                instructions.push(InstructionBlock::new(
                    InstructionKind::Developer,
                    self.name,
                    10,
                ));
                Ok(HookResponse::ModelContext {
                    instructions,
                    messages: request.messages,
                })
            }
            _ => Ok(HookResponse::Continue),
        }
    }
}
fn chain(
    names: &[(&'static str, bool, bool)],
    token: CancellationToken,
) -> (RuntimeHookChain, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(vec![]));
    let handlers = names
        .iter()
        .map(|(name, fail, block)| {
            (
                (*name).into(),
                Arc::new(Handler {
                    name,
                    calls: calls.clone(),
                    fail: *fail,
                    block: *block,
                }) as Arc<dyn HookHandler>,
            )
        })
        .collect();
    (
        RuntimeHookChain::new(
            handlers,
            ExecutionScope::fresh(token),
            Arc::new(NoopExecutionRecorder),
            Arc::new(ModelService::new(Arc::new(MockModel))),
        ),
        calls,
    )
}
fn input(event: HookEvent) -> HookInput {
    HookInput {
        event,
        attribution: crate::contracts::ExecutionAttribution::detached(new_execution_id()),
        cwd: "/tmp".into(),
    }
}
fn bound_input(chain: &RuntimeHookChain, event: HookEvent) -> HookInput {
    let mut input = input(event);
    input.attribution.execution_id = chain.scope.execution_id;
    input
}
fn model_event() -> HookEvent {
    HookEvent::BeforeModel {
        origin: ModelCallOrigin::Direct,
        request: CanonicalModelRequest::new(ModelRef::new("mock", "x"), vec![]),
    }
}
#[tokio::test]
async fn ordering_changes_the_accepted_model_context() {
    for names in [
        [("A", false, false), ("B", false, false)],
        [("B", false, false), ("A", false, false)],
    ] {
        let (chain, calls) = chain(&names, CancellationToken::new());
        let HookEvent::BeforeModel { request, .. } = chain
            .apply(bound_input(&chain, model_event()))
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            *calls.lock().unwrap(),
            names.iter().map(|n| n.0.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            request
                .instructions
                .iter()
                .map(|i| i.text.as_str())
                .collect::<Vec<_>>(),
            names.iter().map(|n| n.0).collect::<Vec<_>>()
        );
    }
}
#[tokio::test]
async fn failure_and_tool_block_stop_later_handlers() {
    let (failed, calls) = chain(
        &[("A", true, false), ("B", false, false)],
        CancellationToken::new(),
    );
    assert!(
        failed
            .apply(bound_input(&failed, model_event()))
            .await
            .is_err()
    );
    assert_eq!(*calls.lock().unwrap(), vec!["A"]);
    let (blocked, calls) = chain(
        &[("A", false, true), ("B", false, false)],
        CancellationToken::new(),
    );
    let event = HookEvent::BeforeTool {
        call: ToolCall::new(new_call_id(), "read", serde_json::json!({})),
        spec: None,
        blocked: None,
    };
    assert!(matches!(
        blocked.apply(bound_input(&blocked, event)).await.unwrap(),
        HookEvent::BeforeTool {
            blocked: Some(_),
            ..
        }
    ));
    assert_eq!(*calls.lock().unwrap(), vec!["A"]);
}
#[tokio::test]
async fn canceled_execution_still_delivers_settled_cleanup_and_notifications_continue() {
    let token = CancellationToken::new();
    token.cancel();
    let (canceled, calls) = chain(&[("A", false, false)], token);
    assert!(
        canceled
            .apply(bound_input(&canceled, model_event()))
            .await
            .is_err()
    );
    assert!(calls.lock().unwrap().is_empty());
    let settled = HookEvent::TurnSettled {
        status: crate::contracts::HookTurnStatus::Canceled,
        output: None,
        error: Some("canceled".into()),
    };
    canceled
        .apply(bound_input(&canceled, settled.clone()))
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["A"]);
    let (notifications, calls) = chain(
        &[("A", true, false), ("B", false, false)],
        CancellationToken::new(),
    );
    assert_eq!(
        notifications
            .apply(bound_input(&notifications, settled.clone()))
            .await
            .unwrap(),
        settled
    );
    assert_eq!(*calls.lock().unwrap(), vec!["A", "B"]);
}

struct InvalidContext;
#[async_trait]
impl HookHandler for InvalidContext {
    async fn invoke(&self, _: HookInput, _: CancellationToken) -> Result<HookResponse> {
        let message = crate::model_standard::CanonicalMessage::text(
            crate::model_standard::MessageRole::User,
            "duplicate",
        );
        Ok(HookResponse::ModelContext {
            messages: vec![message.clone(), message],
            instructions: vec![],
        })
    }
}
#[tokio::test]
async fn invalid_context_and_foreign_attribution_are_rejected_before_later_handlers() {
    let (mut chain, calls) = chain(&[("later", false, false)], CancellationToken::new());
    assert!(chain.apply(input(model_event())).await.is_err());
    assert!(calls.lock().unwrap().is_empty());
    chain
        .handlers
        .insert(0, ("invalid".into(), Arc::new(InvalidContext)));
    assert!(
        chain
            .apply(bound_input(&chain, model_event()))
            .await
            .unwrap_err()
            .to_string()
            .contains("duplicate message")
    );
    assert!(calls.lock().unwrap().is_empty());
}
