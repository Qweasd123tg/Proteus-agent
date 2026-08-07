use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, anyhow, bail};
use proteus_contracts::{
    contracts::{
        PROCESS_MEMORY_RECALL_METHOD, PROCESS_MEMORY_REMEMBER_METHOD, PROCESS_MODULE_CANCEL_METHOD,
        PROCESS_MODULE_CANCELLED_CODE, PROCESS_MODULE_INITIALIZE_METHOD,
        PROCESS_MODULE_PROTOCOL_VERSION, PROCESS_POLICY_EVALUATE_METHOD,
        PROCESS_POLICY_VISIBILITY_METHOD, PROCESS_TOOL_INVOKE_METHOD, PROCESS_TOOL_LIST_METHOD,
        ProcessCompactionResponse, ProcessContextChunksResponse, ProcessContextInput,
        ProcessContextProviderInput, ProcessContextResponse, ProcessMemoryRecallInput,
        ProcessMemoryRecallResponse, ProcessMemoryRememberInput, ProcessMemoryRememberResponse,
        ProcessModuleInitialize, ProcessModuleManifest, ProcessPatchInput, ProcessPatchResponse,
        ProcessPolicyEvaluateInput, ProcessPolicyResponse, ProcessPolicyVisibilityInput,
        ProcessRendererInput, ProcessRendererResponse, ProcessSearchResponse,
        ProcessToolExposureInput, ProcessToolExposureResponse, ProcessToolInvokeInput,
        ProcessToolInvokeResponse, ProcessToolListResponse, ProcessWorkflowInput,
        ProcessWorkflowResponse, WorkflowOutput,
    },
    domain::ToolSpec,
    process_module::{
        ContextBuilderModuleInput, ToolModuleInvocationContext, WorkflowModuleInput,
        WorkflowModuleOutput, WorkflowModuleRuntimeInfo,
    },
};
use proteus_module_protocol::{ProcessModuleRpcError, process_contract_authority};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    hosts::{
        CompactorHostBridge, ContextHostBridge, HostBridge, ToolHostBridge, WorkflowHostBridge,
    },
    registry::CollectedModules,
    transport::{SharedTransport, Transport},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

struct Worker {
    binding: ProcessModuleInitialize,
    modules: CollectedModules,
    bridge: HostBridge,
}

#[derive(Serialize)]
struct PolicyContextWire {
    cwd: String,
    tool_spec: Option<ToolSpec>,
    config: Value,
    granted_permissions: Vec<String>,
}

#[derive(Serialize)]
struct PolicyVisibilityContextWire {
    cwd: String,
    tool_spec: ToolSpec,
    config: Value,
}

pub fn run() -> Result<()> {
    let canceled = Arc::new(AtomicBool::new(false));
    let transport: SharedTransport = Arc::new(Mutex::new(Transport::new(Arc::clone(&canceled))));
    let initialize_value =
        read(&transport)?.ok_or_else(|| anyhow!("missing initialize request"))?;
    let initialize_request: RpcRequest =
        serde_json::from_value(initialize_value).context("invalid initialize JSON-RPC request")?;
    if initialize_request.jsonrpc != "2.0"
        || initialize_request.method != PROCESS_MODULE_INITIALIZE_METHOD
    {
        bail!("first request must be JSON-RPC initialize");
    }
    let initialize_id = initialize_request
        .id
        .clone()
        .ok_or_else(|| anyhow!("initialize request must have an id"))?;
    let binding: ProcessModuleInitialize = serde_json::from_value(initialize_request.params)
        .context("invalid process module initialize params")?;
    validate_initialize(&binding)?;
    let modules = CollectedModules::load(
        &binding.slot,
        &binding.module_id,
        binding.module_config.clone(),
    )?;
    let authority = process_contract_authority(&binding.slot, &binding.contract_version)
        .ok_or_else(|| anyhow!("unknown process contract"))?;
    write(
        &transport,
        &json!({
            "jsonrpc": "2.0",
            "id": initialize_id,
            "result": ProcessModuleManifest {
                protocol_version: PROCESS_MODULE_PROTOCOL_VERSION.to_owned(),
                slot: binding.slot.clone(),
                module_id: binding.module_id.clone(),
                contract_version: binding.contract_version.clone(),
                composition: authority.composition,
                module_features: Vec::new(),
            }
        }),
    )?;

    let worker = Worker {
        binding,
        modules,
        bridge: HostBridge::new(Arc::clone(&transport), Arc::clone(&canceled)),
    };
    loop {
        let Some(value) = read(&transport)? else {
            return Ok(());
        };
        let request: RpcRequest =
            serde_json::from_value(value).context("invalid JSON-RPC request")?;
        if request.jsonrpc != "2.0" {
            bail!("unsupported JSON-RPC version");
        }
        if request.method == PROCESS_MODULE_CANCEL_METHOD {
            canceled.store(true, Ordering::SeqCst);
            continue;
        }
        let id = request
            .id
            .ok_or_else(|| anyhow!("module invocation must have an id"))?;
        worker.bridge.reset_cancellation();
        let result = worker.dispatch(&request.method, request.params);
        let response = if worker.bridge.is_cancelled() {
            rpc_error(
                id,
                ProcessModuleRpcError::new(PROCESS_MODULE_CANCELLED_CODE, "canceled"),
            )
        } else {
            match result {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(error) => rpc_error(
                    id,
                    ProcessModuleRpcError::new(-32_000, format!("{error:#}")),
                ),
            }
        };
        write(&transport, &response)?;
    }
}

fn validate_initialize(binding: &ProcessModuleInitialize) -> Result<()> {
    if binding.protocol_version != PROCESS_MODULE_PROTOCOL_VERSION {
        bail!("unsupported process protocol version");
    }
    let authority = process_contract_authority(&binding.slot, &binding.contract_version)
        .ok_or_else(|| {
            anyhow!(
                "unknown process contract {}/{}",
                binding.slot,
                binding.contract_version
            )
        })?;
    if binding.composition != authority.composition {
        bail!("composition mismatch");
    }
    if binding.host_features != authority.host_features {
        bail!("host feature mismatch");
    }
    Ok(())
}

impl Worker {
    fn dispatch(&self, method: &str, params: Value) -> Result<Value> {
        let authority =
            process_contract_authority(&self.binding.slot, &self.binding.contract_version)
                .expect("validated process authority");
        if !authority.allows_module_method(method) {
            bail!(
                "method {method:?} is not allowed for slot {}",
                self.binding.slot
            );
        }
        match self.binding.slot.as_str() {
            "tool" => self.tool(method, params),
            "search" => self.search(method, params),
            "memory" => self.memory(method, params),
            "patch" => self.patch(method, params),
            "policy" => self.policy(method, params),
            "tool_exposure" => self.tool_exposure(method, params),
            "renderer" => self.renderer(method, params),
            "context" => self.context(method, params),
            "context_provider" => self.context_provider(method, params),
            "compactor" => self.compactor(method, params),
            "workflow" => self.workflow(method, params),
            slot => bail!("reference worker does not dispatch slot {slot:?}"),
        }
    }

    fn tool(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            PROCESS_TOOL_LIST_METHOD => {
                if !params.is_null() {
                    bail!("tool list params must be null");
                }
                let specs = self
                    .modules
                    .tools
                    .iter()
                    .map(|tool| {
                        let json = tool.spec_json();
                        serde_json::from_str::<ToolSpec>(json.as_str()).map_err(Into::into)
                    })
                    .collect::<Result<Vec<_>>>()?;
                encode(ProcessToolListResponse::new(specs))
            }
            PROCESS_TOOL_INVOKE_METHOD => {
                let input: ProcessToolInvokeInput = decode(params)?;
                let tool = self
                    .modules
                    .tools
                    .iter()
                    .find(|tool| {
                        serde_json::from_str::<ToolSpec>(tool.spec_json().as_str())
                            .is_ok_and(|spec| spec.name == input.call.name)
                    })
                    .ok_or_else(|| anyhow!("tool module does not provide {}", input.call.name))?;
                let call_json = serde_json::to_string(&input.call)?;
                let context_json = serde_json::to_string(&ToolModuleInvocationContext {
                    cwd: input.cwd,
                    owner: input.owner,
                    config: self.binding.module_config.clone(),
                })?;
                let mut host = ToolHostBridge(self.bridge.clone());
                let output = tool.invoke_json(call_json, context_json, &mut host)?;
                let result = serde_json::from_str(output.as_str())?;
                encode(ProcessToolInvokeResponse::new(result))
            }
            _ => unreachable!(),
        }
    }

    fn search(&self, _method: &str, params: Value) -> Result<Value> {
        let backend = self
            .modules
            .searches
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("search module was not registered"))?;
        let output = backend.search_json(serde_json::to_string(&params)?)?;
        encode(ProcessSearchResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn memory(&self, method: &str, params: Value) -> Result<Value> {
        let store = self
            .modules
            .memories
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("memory module was not registered"))?;
        match method {
            PROCESS_MEMORY_REMEMBER_METHOD => {
                let input: ProcessMemoryRememberInput = decode(params)?;
                store.remember_json(serde_json::to_string(&input.item)?)?;
                encode(ProcessMemoryRememberResponse::new(()))
            }
            PROCESS_MEMORY_RECALL_METHOD => {
                let input: ProcessMemoryRecallInput = decode(params)?;
                let output = store.recall_json(serde_json::to_string(&input.query)?)?;
                encode(ProcessMemoryRecallResponse::new(serde_json::from_str(
                    output.as_str(),
                )?))
            }
            _ => unreachable!(),
        }
    }

    fn patch(&self, _method: &str, params: Value) -> Result<Value> {
        let input: ProcessPatchInput = decode(params)?;
        let applier = self
            .modules
            .patches
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("patch module was not registered"))?;
        let output = applier.apply_json(
            serde_json::to_string(&input.patch)?,
            input.cwd.to_string_lossy().into_owned(),
        )?;
        encode(ProcessPatchResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn policy(&self, method: &str, params: Value) -> Result<Value> {
        let policy = self
            .modules
            .policies
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("policy module was not registered"))?;
        let output = match method {
            PROCESS_POLICY_EVALUATE_METHOD => {
                let input: ProcessPolicyEvaluateInput = decode(params)?;
                policy.evaluate_json(
                    serde_json::to_string(&input.call)?,
                    serde_json::to_string(&PolicyContextWire {
                        cwd: input.cwd.to_string_lossy().into_owned(),
                        tool_spec: input.tool_spec,
                        config: self.binding.module_config.clone(),
                        granted_permissions: input.granted_permissions,
                    })?,
                )
            }
            PROCESS_POLICY_VISIBILITY_METHOD => {
                let input: ProcessPolicyVisibilityInput = decode(params)?;
                policy.evaluate_visibility_json(serde_json::to_string(
                    &PolicyVisibilityContextWire {
                        cwd: input.cwd.to_string_lossy().into_owned(),
                        tool_spec: input.tool_spec,
                        config: self.binding.module_config.clone(),
                    },
                )?)
            }
            _ => unreachable!(),
        };
        let output = output?;
        encode(ProcessPolicyResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn tool_exposure(&self, _method: &str, params: Value) -> Result<Value> {
        let input: ProcessToolExposureInput = decode(params)?;
        let exposure = self
            .modules
            .exposures
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("tool exposure module was not registered"))?;
        let output = exposure.select_json(serde_json::to_string(&input.input)?)?;
        encode(ProcessToolExposureResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn renderer(&self, _method: &str, params: Value) -> Result<Value> {
        let input: ProcessRendererInput = decode(params)?;
        let renderer = self
            .modules
            .renderers
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("renderer module was not registered"))?;
        let output = renderer.render_json(serde_json::to_string(&input.output)?)?;
        encode(ProcessRendererResponse::new(output))
    }

    fn context(&self, _method: &str, params: Value) -> Result<Value> {
        let input: ProcessContextInput = decode(params)?;
        let builder = self
            .modules
            .contexts
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("context module was not registered"))?;
        let module_input = ContextBuilderModuleInput {
            task: input.task,
            config: self.binding.module_config.clone(),
        };
        let mut host = ContextHostBridge(self.bridge.clone());
        let output = builder.build_json(serde_json::to_string(&module_input)?, &mut host)?;
        encode(ProcessContextResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn context_provider(&self, _method: &str, params: Value) -> Result<Value> {
        let input: ProcessContextProviderInput = decode(params)?;
        let provider = self
            .modules
            .context_providers
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("context provider module was not registered"))?;
        let output = provider.provide_json(serde_json::to_string(&input)?)?;
        encode(ProcessContextChunksResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn compactor(&self, _method: &str, params: Value) -> Result<Value> {
        let mut input: proteus_contracts::contracts::CompactionInput = decode(params)?;
        input.config = self.binding.module_config.clone();
        let compactor = self
            .modules
            .compactors
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("compactor module was not registered"))?;
        let mut host = CompactorHostBridge(self.bridge.clone());
        let output = compactor.compact_json(serde_json::to_string(&input)?, &mut host)?;
        encode(ProcessCompactionResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn workflow(&self, _method: &str, params: Value) -> Result<Value> {
        let input: ProcessWorkflowInput = decode(params)?;
        let workflow = self
            .modules
            .workflows
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("workflow module was not registered"))?;
        let module_input = WorkflowModuleInput {
            task: input.task,
            history: input.history,
            config: self.binding.module_config.clone(),
            runtime: WorkflowModuleRuntimeInfo {
                session_id: input.runtime.session_id,
                thread_id: input.runtime.thread_id,
                turn_id: input.runtime.turn_id,
                model_ref: input.runtime.model_ref,
                instructions: input.runtime.instructions,
                reasoning: input.runtime.reasoning,
                max_input_tokens: input.runtime.max_input_tokens,
                model_timeout_ms: input.runtime.model_timeout_ms,
                context_timeout_ms: input.runtime.context_timeout_ms,
                workflow_timeout_ms: input.runtime.workflow_timeout_ms,
            },
        };
        let mut host = WorkflowHostBridge(self.bridge.clone());
        let output = workflow.run_json(serde_json::to_string(&module_input)?, &mut host)?;
        let output: WorkflowModuleOutput = serde_json::from_str(output.as_str())?;
        let mut result = WorkflowOutput::new(output.output, output.new_messages)
            .with_compactions(output.compactions);
        if let Some(messages) = output.history_replacement {
            result = result.with_history_replacement(messages);
        }
        encode(ProcessWorkflowResponse::new(result))
    }
}

fn read(transport: &SharedTransport) -> Result<Option<Value>> {
    transport
        .lock()
        .map_err(|_| anyhow!("worker transport lock poisoned"))?
        .read()
}

fn write(transport: &SharedTransport, value: &Value) -> Result<()> {
    transport
        .lock()
        .map_err(|_| anyhow!("worker transport lock poisoned"))?
        .write(value)
}

fn rpc_error(id: Value, error: ProcessModuleRpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(Into::into)
}

fn encode<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(Into::into)
}
