use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use proteus_module_protocol::v3::{
    AsyncHostRequestDispatcher, CancelCause, ComponentBroker, ComponentBrokerError,
    InvocationHandle, InvocationTerminal, NoAsyncHostRequests,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::time::{MissedTickBehavior, interval};

use super::{
    ProcessExportConfig, ProcessInvocationError, ProcessInvocationFailure,
    invocation_scope::{current_parent, scoped_dispatcher},
};

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Typed view of one export on a shared multiplexed process component broker.
pub struct ProcessExportClient {
    component_id: String,
    module_id: String,
    target: proteus_contracts::contracts::ProcessComponentExportRef,
    timeout: Duration,
    broker: Arc<ComponentBroker>,
}

impl ProcessExportClient {
    pub fn connect(
        slot: &str,
        contract_version: &str,
        config: ProcessExportConfig,
        workspace: &Path,
        default_timeout_ms: u64,
    ) -> Result<Self> {
        config.validate_for(slot, default_timeout_ms)?;
        if config.binding().contract_version != contract_version {
            bail!(
                "component {:?} export {}/{} uses contract {:?}, expected {:?}",
                config.component_id(),
                config.slot(),
                config.module_id(),
                config.binding().contract_version,
                contract_version
            );
        }
        let timeout = config.timeout(default_timeout_ms);
        let component_id = config.component_id().to_owned();
        let module_id = config.module_id().to_owned();
        let target = config.binding().export_ref();
        let broker = config.connect(workspace)?;
        Ok(Self {
            component_id,
            module_id,
            target,
            timeout,
            broker,
        })
    }

    pub async fn invoke<P, R>(&self, method: &str, params: &P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = self.encode(method, params)?;
        let mut handle = self
            .start(method, value, Arc::new(NoAsyncHostRequests))
            .await
            .with_context(|| self.invocation_context(method))?;
        let terminal = handle
            .result()
            .await
            .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    pub async fn invoke_with_dispatcher<P, R>(
        &self,
        method: &str,
        params: &P,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
    ) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = self.encode(method, params)?;
        let mut handle = self
            .start(method, value, dispatcher)
            .await
            .with_context(|| self.invocation_context(method))?;
        let terminal = handle
            .result()
            .await
            .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    pub async fn invoke_with_dispatcher_and_cancel_check<P, R, F>(
        &self,
        method: &str,
        params: &P,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
        is_cancelled: F,
    ) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
        F: Fn() -> bool,
    {
        if is_cancelled() {
            bail!("process module {:?}: {method} was canceled", self.module_id);
        }
        let value = self.encode(method, params)?;
        let mut handle = self
            .start(method, value, dispatcher)
            .await
            .with_context(|| self.invocation_context(method))?;
        let cancel = handle.cancel_handle();
        let mut terminal = Box::pin(handle.result());
        let mut cancellation_poll = interval(CANCEL_POLL_INTERVAL);
        cancellation_poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let terminal = loop {
            tokio::select! {
                result = &mut terminal => break result,
                _ = cancellation_poll.tick() => {
                    if is_cancelled() {
                        cancel.cancel(CancelCause::User)
                            .with_context(|| self.invocation_context(method))?;
                        break terminal.await;
                    }
                }
            }
        }
        .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    /// Used while a catalog is being built, before runtime traffic can start.
    pub fn invoke_bootstrap<P, R>(&self, method: &str, params: &P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = self.encode(method, params)?;
        let terminal = self
            .broker
            .invoke_bootstrap(&self.target, method, value, self.timeout)
            .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    /// Used only by callback-free synchronous slot traits.
    pub fn invoke_blocking<P, R>(&self, method: &str, params: &P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = self.encode(method, params)?;
        let terminal = match current_parent(&self.broker) {
            Some(parent) => self.broker.invoke_nested_blocking(
                &parent,
                &self.target,
                method,
                value,
                self.timeout,
            ),
            None => self
                .broker
                .invoke_blocking(&self.target, method, value, self.timeout),
        }
        .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    async fn start(
        &self,
        method: &str,
        params: Value,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
    ) -> Result<InvocationHandle, ComponentBrokerError> {
        let dispatcher = scoped_dispatcher(&self.broker, dispatcher);
        match current_parent(&self.broker) {
            Some(parent) => {
                self.broker
                    .start_nested_invocation(
                        &parent,
                        &self.target,
                        method,
                        params,
                        self.timeout,
                        dispatcher,
                    )
                    .await
            }
            None => {
                self.broker
                    .start_invocation_with_dispatcher(
                        &self.target,
                        method,
                        params,
                        self.timeout,
                        dispatcher,
                    )
                    .await
            }
        }
    }

    fn encode<P: Serialize>(&self, method: &str, params: &P) -> Result<Value> {
        serde_json::to_value(params).with_context(|| {
            format!(
                "process module {:?}: failed to serialize {method} request",
                self.module_id
            )
        })
    }

    fn invocation_context(&self, method: &str) -> String {
        format!(
            "process component {:?} export {:?}: {method} invocation failed",
            self.component_id, self.module_id
        )
    }

    fn decode<R: DeserializeOwned>(&self, method: &str, terminal: InvocationTerminal) -> Result<R> {
        let value = terminal_value(terminal, &self.module_id, method)?;
        match serde_json::from_value(value) {
            Ok(response) => Ok(response),
            Err(error) => {
                let _ = self.broker.reset();
                Err(error).with_context(|| {
                    format!(
                        "process component {:?} export {:?}: {method} returned an invalid response",
                        self.component_id, self.module_id
                    )
                })
            }
        }
    }
}

fn terminal_value(terminal: InvocationTerminal, module_id: &str, method: &str) -> Result<Value> {
    let failure = match terminal {
        InvocationTerminal::Success(value) => return Ok(value),
        InvocationTerminal::ModuleError(error) => ProcessInvocationFailure::Module(error),
        InvocationTerminal::Canceled => ProcessInvocationFailure::Canceled,
        InvocationTerminal::TimedOut => ProcessInvocationFailure::TimedOut,
        InvocationTerminal::ComponentLost(failure) => {
            ProcessInvocationFailure::ComponentLost(failure)
        }
    };
    Err(ProcessInvocationError::new(module_id, method, failure).into())
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use proteus_module_protocol::{
        ProcessExportBinding, ProcessModuleRpcError,
        v3::{
            AsyncHostRequestDispatcher, ComponentFailure, ComponentHostRequest, HostRequestFuture,
            InvocationTerminal,
        },
    };
    use serde_json::{Value, json};

    use super::{
        ProcessExportClient, ProcessInvocationError, ProcessInvocationFailure, terminal_value,
    };
    use crate::{
        contracts::{
            PROCESS_CONTEXT_BUILD_METHOD, PROCESS_CONTEXT_CONTRACT_VERSION,
            PROCESS_POLICY_CONTRACT_VERSION, PROCESS_POLICY_EVALUATE_METHOD,
            PROCESS_SEARCH_CONTRACT_VERSION, PROCESS_SEARCH_METHOD,
        },
        process_adapters::{ProcessComponentConfig, ProcessComponentLauncher},
    };

    #[test]
    fn invocation_terminals_preserve_their_failure_class_in_adapter_errors() {
        let cases = [
            (
                InvocationTerminal::Canceled,
                ProcessInvocationFailure::Canceled,
                "was canceled",
            ),
            (
                InvocationTerminal::TimedOut,
                ProcessInvocationFailure::TimedOut,
                "timed out",
            ),
            (
                InvocationTerminal::ComponentLost(ComponentFailure::Protocol),
                ProcessInvocationFailure::ComponentLost(ComponentFailure::Protocol),
                "lost its component: Protocol",
            ),
            (
                InvocationTerminal::ModuleError(ProcessModuleRpcError::new(-32000, "broken")),
                ProcessInvocationFailure::Module(ProcessModuleRpcError::new(-32000, "broken")),
                "returned an error",
            ),
        ];

        for (terminal, expected_failure, expected_message) in cases {
            let error = terminal_value(terminal, "probe", "run")
                .expect_err("non-success terminal must remain an error");
            let typed = error
                .downcast_ref::<ProcessInvocationError>()
                .expect("terminal class remains machine-readable");
            assert_eq!(typed.module_id(), "probe");
            assert_eq!(typed.method(), "run");
            assert_eq!(typed.failure(), &expected_failure);
            assert!(format!("{error:#}").contains(expected_message), "{error:#}");
        }
    }

    struct NestedProbeDispatcher {
        search: Arc<ProcessExportClient>,
        policy: Arc<ProcessExportClient>,
    }

    impl AsyncHostRequestDispatcher for NestedProbeDispatcher {
        fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
            let search = Arc::clone(&self.search);
            let outer_id = request.invocation.id().to_owned();
            let blocking: Result<Value, ProcessModuleRpcError> = self
                .policy
                .invoke_blocking(PROCESS_POLICY_EVALUATE_METHOD, &json!({"op": "lineage"}))
                .map_err(callback_error);
            Box::pin(async move {
                let asynchronous: Value = search
                    .invoke(PROCESS_SEARCH_METHOD, &json!({"op": "lineage"}))
                    .await
                    .map_err(callback_error)?;
                let blocking = blocking?;
                Ok(json!({
                    "outer_id": outer_id,
                    "asynchronous": asynchronous,
                    "blocking": blocking,
                }))
            })
        }
    }

    fn callback_error(error: anyhow::Error) -> ProcessModuleRpcError {
        ProcessModuleRpcError::new(-32_100, format!("nested probe failed: {error:#}"))
    }

    fn fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../proteus-module-protocol/tests/fixtures/multiplex_worker.py")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn callback_reentry_preserves_lineage_for_async_and_blocking_same_broker_calls() {
        let workspace = tempfile::tempdir().expect("workspace");
        let component: ProcessComponentConfig = serde_json::from_value(json!({
            "command": "python3",
            "args": [fixture_path()],
            "handshake_timeout_ms": 3_000,
            "exports": {
                "context": {"scope.context": {"timeout_ms": 3_000}},
                "search": {"scope.search": {"timeout_ms": 3_000}},
                "policy": {"scope.policy": {"timeout_ms": 3_000}}
            }
        }))
        .expect("component config");
        let bindings = [
            ProcessExportBinding::new(
                "context",
                "scope.context",
                PROCESS_CONTEXT_CONTRACT_VERSION,
                json!({}),
            )
            .expect("context binding"),
            ProcessExportBinding::new(
                "search",
                "scope.search",
                PROCESS_SEARCH_CONTRACT_VERSION,
                json!({}),
            )
            .expect("search binding"),
            ProcessExportBinding::new(
                "policy",
                "scope.policy",
                PROCESS_POLICY_CONTRACT_VERSION,
                json!({}),
            )
            .expect("policy binding"),
        ];
        let launcher = ProcessComponentLauncher::new("scope-probe", component, bindings.to_vec())
            .expect("launcher");
        let context = Arc::new(
            ProcessExportClient::connect(
                "context",
                PROCESS_CONTEXT_CONTRACT_VERSION,
                launcher
                    .export("context", "scope.context")
                    .expect("context export"),
                workspace.path(),
                3_000,
            )
            .expect("context client"),
        );
        let search = Arc::new(
            ProcessExportClient::connect(
                "search",
                PROCESS_SEARCH_CONTRACT_VERSION,
                launcher
                    .export("search", "scope.search")
                    .expect("search export"),
                workspace.path(),
                3_000,
            )
            .expect("search client"),
        );
        let policy = Arc::new(
            ProcessExportClient::connect(
                "policy",
                PROCESS_POLICY_CONTRACT_VERSION,
                launcher
                    .export("policy", "scope.policy")
                    .expect("policy export"),
                workspace.path(),
                3_000,
            )
            .expect("policy client"),
        );

        let response: Value = context
            .invoke_with_dispatcher(
                PROCESS_CONTEXT_BUILD_METHOD,
                &json!({"op": "callback"}),
                Arc::new(NestedProbeDispatcher {
                    search: Arc::clone(&search),
                    policy,
                }),
            )
            .await
            .expect("outer invocation");
        let callback = &response["value"]["callback_result"];
        let outer_id = callback["outer_id"].as_str().expect("outer id");
        for kind in ["asynchronous", "blocking"] {
            let lineage = &callback[kind]["value"];
            assert_eq!(lineage["root_invocation_id"], outer_id, "{kind}");
            assert_eq!(lineage["parent_invocation_id"], outer_id, "{kind}");
            assert_eq!(lineage["depth"], 1, "{kind}");
        }

        let standalone: Value = search
            .invoke(PROCESS_SEARCH_METHOD, &json!({"op": "lineage"}))
            .await
            .expect("standalone search");
        let lineage = &standalone["value"];
        assert_eq!(lineage["parent_invocation_id"], Value::Null);
        assert_eq!(lineage["depth"], 0);
        assert!(
            lineage["root_invocation_id"]
                .as_str()
                .is_some_and(|id| id.starts_with("h:")),
            "standalone invocation must own a host-generated root id"
        );
    }
}
