use anyhow::{Result, anyhow, bail};
use proteus_contracts::{
    contracts::{
        PROCESS_MEMORY_RECALL_METHOD, PROCESS_MEMORY_REMEMBER_METHOD,
        PROCESS_POLICY_EVALUATE_METHOD, PROCESS_POLICY_VISIBILITY_METHOD,
        PROCESS_TOOL_INVOKE_METHOD, PROCESS_TOOL_LIST_METHOD, ProcessCompactionResponse,
        ProcessComponentExportInitialize, ProcessComponentExportManifest,
        ProcessContextChunksResponse, ProcessContextInput, ProcessContextProviderInput,
        ProcessContextResponse, ProcessMemoryRecallInput, ProcessMemoryRecallResponse,
        ProcessMemoryRememberInput, ProcessMemoryRememberResponse, ProcessPatchInput,
        ProcessPatchResponse, ProcessPolicyEvaluateInput, ProcessPolicyResponse,
        ProcessPolicyVisibilityInput, ProcessRendererInput, ProcessRendererResponse,
        ProcessSearchResponse, ProcessToolExposureInput, ProcessToolExposureResponse,
        ProcessToolInvokeInput, ProcessToolInvokeResponse, ProcessToolListResponse,
        ProcessWorkflowInput, ProcessWorkflowResponse, WorkflowOutput,
    },
    domain::ToolSpec,
    process_module::{
        ContextBuilderModuleInput, ToolModuleInvocationContext, WorkflowModuleInput,
        WorkflowModuleOutput, WorkflowModuleRuntimeInfo,
    },
};
use proteus_module_protocol::process_contract_authority;
use serde::Serialize;
use serde_json::Value;

use crate::{
    hosts::{
        CompactorHostBridge, ContextHostBridge, HostBridge, ToolHostBridge, WorkflowHostBridge,
    },
    registry::CollectedModules,
};

pub(crate) struct ExportWorker {
    binding: ProcessComponentExportInitialize,
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

impl ExportWorker {
    pub(crate) fn load(
        binding: ProcessComponentExportInitialize,
        bridge: HostBridge,
    ) -> Result<Self> {
        let modules = CollectedModules::load(
            &binding.slot,
            &binding.module_id,
            binding.module_config.clone(),
        )?;
        Ok(Self {
            binding,
            modules,
            bridge,
        })
    }

    pub(crate) fn manifest(&self) -> ProcessComponentExportManifest {
        let authority =
            process_contract_authority(&self.binding.slot, &self.binding.contract_version)
                .expect("validated process authority");
        ProcessComponentExportManifest {
            slot: self.binding.slot.clone(),
            module_id: self.binding.module_id.clone(),
            contract_version: self.binding.contract_version.clone(),
            composition: authority.composition,
            module_features: Vec::new(),
        }
    }

    pub(crate) fn dispatch(&self, method: &str, params: Value) -> Result<Value> {
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
            "search" => self.search(params),
            "memory" => self.memory(method, params),
            "patch" => self.patch(params),
            "policy" => self.policy(method, params),
            "tool_exposure" => self.tool_exposure(params),
            "renderer" => self.renderer(params),
            "context" => self.context(params),
            "context_provider" => self.context_provider(params),
            "compactor" => self.compactor(params),
            "workflow" => self.workflow(params),
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

    fn search(&self, params: Value) -> Result<Value> {
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

    fn patch(&self, params: Value) -> Result<Value> {
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
        }?;
        encode(ProcessPolicyResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn tool_exposure(&self, params: Value) -> Result<Value> {
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

    fn renderer(&self, params: Value) -> Result<Value> {
        let input: ProcessRendererInput = decode(params)?;
        let renderer = self
            .modules
            .renderers
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("renderer module was not registered"))?;
        let output = renderer.render_json(serde_json::to_string(&input.output)?)?;
        encode(ProcessRendererResponse::new(output))
    }

    fn context(&self, params: Value) -> Result<Value> {
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

    fn context_provider(&self, params: Value) -> Result<Value> {
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

    fn compactor(&self, params: Value) -> Result<Value> {
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

    fn workflow(&self, params: Value) -> Result<Value> {
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

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(Into::into)
}

fn encode<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(Into::into)
}
