use super::*;
use crate::{
    contracts::{
        ApprovalPolicy, PolicyContext, PolicyVisibilityContext, Tool, ToolContext, ToolRegistry,
    },
    core::{BoundTools, HeadlessApprovalTransport, ToolExecutionBinding},
    domain::{
        HostedToolConfig, PolicyDecision, ToolCall, ToolResult, ToolSafety, ToolSpec, ToolSurface,
        WebSearchHostedToolConfig,
    },
};
use serde_json::json;

struct ProbeTool(ToolSpec);
#[async_trait]
impl Tool for ProbeTool {
    fn spec(&self) -> ToolSpec {
        self.0.clone()
    }
    async fn invoke(&self, _: &ToolCall, _: ToolContext) -> Result<ToolResult> {
        panic!("must not execute")
    }
}

struct ReadOnlyPolicy;
impl ApprovalPolicy for ReadOnlyPolicy {
    fn evaluate(&self, _: &ToolCall, _: &PolicyContext) -> PolicyDecision {
        panic!("visibility only")
    }
    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision {
        if ctx.tool_spec.safety == ToolSafety::ReadOnly {
            PolicyDecision::Allow
        } else {
            PolicyDecision::Deny {
                reason: "blocked".into(),
            }
        }
    }
}

#[tokio::test]
async fn model_request_cannot_redefine_or_expose_policy_hidden_tools() {
    let cwd = std::env::current_dir().unwrap();
    let local = ToolSpec::new(
        "web_search",
        "local probe",
        json!({"type":"object"}),
        ToolSafety::ReadOnly,
    );
    let mut hosted = local.clone();
    hosted.safety = ToolSafety::Network;
    hosted.surface = ToolSurface::provider_hosted(HostedToolConfig::WebSearch {
        config: WebSearchHostedToolConfig::default(),
    });
    for registered in [local.clone(), hosted.clone()] {
        let adapter = Arc::new(ImmediateAdapter::new());
        let mut registry = ToolRegistry::new();
        registry.register(ProbeTool(registered.clone())).unwrap();
        let scope = ExecutionScope::fresh(CancellationToken::new());
        let attribution = ExecutionAttribution::detached(scope.execution_id);
        let tools = BoundTools::new(
            registry,
            Arc::new(ReadOnlyPolicy),
            Arc::new(HeadlessApprovalTransport),
            Arc::default(),
            ToolExecutionBinding::detached(scope.clone()),
        );
        let model = BoundModel::new(
            Arc::new(ModelService::new(adapter.clone())),
            ModelExecutionBinding::detached(scope),
            0,
        )
        .with_tool_authority(tools)
        .with_hooks(
            Arc::new(crate::contracts::NoExecutionHooks),
            attribution,
            cwd.clone(),
        );
        let request = CanonicalModelRequest::new(
            ModelRef::new("probe", "probe"),
            vec![CanonicalMessage::text(MessageRole::User, "probe")],
        )
        .with_tools(vec![hosted.clone()]);
        let error = model.complete(request).await.unwrap_err();
        assert!(
            error.to_string().contains(if registered == local {
                "changed registered tool"
            } else {
                "policy-hidden tool"
            }),
            "{error:#}"
        );
        assert!(adapter.requests.lock().unwrap().is_empty());
        model
            .complete(CanonicalModelRequest::new(
                ModelRef::new("probe", "probe"),
                vec![CanonicalMessage::text(MessageRole::User, "probe")],
            ))
            .await
            .unwrap();
        assert_eq!(adapter.requests.lock().unwrap().len(), 1);
    }
}
