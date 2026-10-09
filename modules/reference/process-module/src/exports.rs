use anyhow::{Result, anyhow, bail};
use proteus_contracts::{
    contracts::{
        PROCESS_MEMORY_RECALL_METHOD, PROCESS_MEMORY_REMEMBER_METHOD,
        PROCESS_POLICY_EVALUATE_METHOD, PROCESS_POLICY_VISIBILITY_METHOD,
        PROCESS_TOOL_INVOKE_METHOD, PROCESS_TOOL_LIST_METHOD, ProcessCompactionResponse,
        ProcessComponentExportInitialize, ProcessComponentExportManifest,
        ProcessContextChunksResponse, ProcessContextInput, ProcessContextResponse,
        ProcessMemoryRecallInput, ProcessMemoryRecallResponse, ProcessMemoryRememberInput,
        ProcessMemoryRememberResponse, ProcessPatchInput, ProcessPatchResponse,
        ProcessPolicyEvaluateInput, ProcessPolicyResponse, ProcessPolicyVisibilityInput,
        ProcessSearchResponse, ProcessToolExposureInput, ProcessToolExposureResponse,
        ProcessToolInvokeInput, ProcessToolInvokeResponse, ProcessToolListResponse,
        ProcessWorkflowInput, ProcessWorkflowResponse, WorkflowOutput,
    },
    domain::ToolSpec,
    process_module::{
        ContextBuilderModuleInput, MemoryModuleInvocationContext, PolicyModuleInvocationContext,
        PolicyModuleVisibilityContext, ToolModuleInvocationContext, WorkflowModuleInput,
        WorkflowModuleOutput,
    },
};
use proteus_module_protocol::process_contract_authority;
use serde_json::Value;

use crate::{
    hosts::{
        CompactorHostBridge, ContextHostBridge, HostBridge, MemoryHostBridge, ToolHostBridge,
        WorkflowHostBridge,
    },
    registry::CollectedModules,
};

pub(crate) struct ModuleExport {
    binding: ProcessComponentExportInitialize,
    modules: CollectedModules,
}

impl ModuleExport {
    pub(crate) fn load(binding: ProcessComponentExportInitialize) -> Result<Self> {
        let modules = CollectedModules::load(
            &binding.slot,
            &binding.module_id,
            binding.module_config.clone(),
        )?;
        Ok(Self { binding, modules })
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
            config_schema: crate::config_schema::describe(
                &self.binding.slot,
                &self.binding.module_id,
                &self.binding.module_config,
            ),
        }
    }

    pub(crate) fn dispatch(
        &self,
        method: &str,
        params: Value,
        bridge: &HostBridge,
    ) -> Result<Value> {
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
            "hook" => self.hook(params),
            "model" => self.model(method, params, bridge),
            "tool" => self.tool(method, params, bridge),
            "search" => self.search(params),
            "memory" => self.memory(method, params, bridge),
            "patch" => self.patch(params),
            "policy" => self.policy(method, params),
            "tool_exposure" => self.tool_exposure(params),
            "context" => self.context(params, bridge),
            "context_provider" => self.context_provider(method, params),
            "compactor" => self.compactor(params, bridge),
            "workflow" => self.workflow(params, bridge),
            slot => bail!("reference-module does not dispatch slot {slot:?}"),
        }
    }

    fn hook(&self, params: Value) -> Result<Value> {
        let input: proteus_contracts::contracts::HookInput = decode(params)?;
        let hook = self
            .modules
            .hooks
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("hook module missing"))?;
        let output = hook.invoke_json(serde_json::to_string(&input)?)?;
        let response: proteus_contracts::contracts::HookResponse = serde_json::from_str(&output)?;
        proteus_contracts::contracts::apply_hook_response(&input.event, &response)?;
        encode(proteus_contracts::contracts::ProcessHookResponse { result: response })
    }

    fn tool(&self, method: &str, params: Value, bridge: &HostBridge) -> Result<Value> {
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
                        Ok(proteus_contracts::contracts::ProcessToolDefinition {
                            spec: serde_json::from_str::<ToolSpec>(json.as_str())?,
                            model_visible: tool.model_visible(),
                            user_command: tool.user_command(),
                        })
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
                    attribution: input.attribution,
                    skills: input.skills,
                    config: self.binding.module_config.clone(),
                })?;
                let mut host = ToolHostBridge(bridge.clone());
                let output = tool.invoke_json(call_json, context_json, &mut host)?;
                let result = serde_json::from_str(output.as_str())?;
                encode(ProcessToolInvokeResponse::new(result))
            }
            _ => unreachable!(),
        }
    }

    fn model(&self, method: &str, params: Value, bridge: &HostBridge) -> Result<Value> {
        use proteus_contracts::contracts::{
            PROCESS_MODEL_CATALOG_METHOD, PROCESS_MODEL_DESCRIBE_METHOD,
            PROCESS_MODEL_QUOTA_METHOD, PROCESS_MODEL_STREAM_METHOD,
        };
        let model = self
            .modules
            .models
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("model module was not registered"))?;
        match method {
            PROCESS_MODEL_DESCRIBE_METHOD => {
                let input: proteus_contracts::contracts::ProcessModelDescribeRequest =
                    decode(params)?;
                encode(model.describe(input.model)?)
            }
            PROCESS_MODEL_CATALOG_METHOD => {
                if !params.is_null() {
                    bail!("model catalog params must be null");
                }
                encode(model.catalog(&crate::hosts::ModelHostBridge(bridge.clone()))?)
            }
            PROCESS_MODEL_QUOTA_METHOD => {
                if !params.is_null() {
                    bail!("model quota params must be null");
                }
                encode(model.quota(&crate::hosts::ModelHostBridge(bridge.clone()))?)
            }
            PROCESS_MODEL_STREAM_METHOD => {
                let input = decode(params)?;
                encode(model.stream(input, &crate::hosts::ModelHostBridge(bridge.clone()))?)
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

    fn memory(&self, method: &str, params: Value, bridge: &HostBridge) -> Result<Value> {
        let store = self
            .modules
            .memories
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("memory module was not registered"))?;
        match method {
            PROCESS_MEMORY_REMEMBER_METHOD => {
                let input: ProcessMemoryRememberInput = decode(params)?;
                let context_json = serde_json::to_string(&MemoryModuleInvocationContext {
                    attribution: input.attribution,
                    config: self.binding.module_config.clone(),
                })?;
                let mut host = MemoryHostBridge(bridge.clone());
                store.remember_json(
                    serde_json::to_string(&input.item)?,
                    context_json,
                    &mut host,
                )?;
                encode(ProcessMemoryRememberResponse::new(()))
            }
            PROCESS_MEMORY_RECALL_METHOD => {
                let input: ProcessMemoryRecallInput = decode(params)?;
                let context_json = serde_json::to_string(&MemoryModuleInvocationContext {
                    attribution: input.attribution,
                    config: self.binding.module_config.clone(),
                })?;
                let mut host = MemoryHostBridge(bridge.clone());
                let output = store.recall_json(
                    serde_json::to_string(&input.query)?,
                    context_json,
                    &mut host,
                )?;
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
                    serde_json::to_string(&PolicyModuleInvocationContext {
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
                    &PolicyModuleVisibilityContext {
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

    fn context(&self, params: Value, bridge: &HostBridge) -> Result<Value> {
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
        let mut host = ContextHostBridge(bridge.clone());
        let output = builder.build_json(serde_json::to_string(&module_input)?, &mut host)?;
        encode(ProcessContextResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn context_provider(&self, method: &str, params: Value) -> Result<Value> {
        let provider = self
            .modules
            .context_providers
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("context provider module was not registered"))?;
        if method == proteus_contracts::contracts::PROCESS_CONTEXT_PROVIDER_CATALOG_METHOD {
            let input = decode(params)?;
            return encode(
                proteus_contracts::contracts::ProcessSkillCatalogResponse::new(
                    provider.skill_catalog(input)?,
                ),
            );
        }
        let input: proteus_contracts::contracts::ProcessContextProviderRequest = decode(params)?;
        let output = provider.provide_json(serde_json::to_string(&input)?)?;
        encode(ProcessContextChunksResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn compactor(&self, params: Value, bridge: &HostBridge) -> Result<Value> {
        let mut input: proteus_contracts::contracts::CompactionInput = decode(params)?;
        input.config = self.binding.module_config.clone();
        let compactor = self
            .modules
            .compactors
            .get(&self.binding.module_id)
            .ok_or_else(|| anyhow!("compactor module was not registered"))?;
        let mut host = CompactorHostBridge(bridge.clone());
        let output = compactor.compact_json(serde_json::to_string(&input)?, &mut host)?;
        encode(ProcessCompactionResponse::new(serde_json::from_str(
            output.as_str(),
        )?))
    }

    fn workflow(&self, params: Value, bridge: &HostBridge) -> Result<Value> {
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
            runtime: input.runtime,
        };
        let mut host = WorkflowHostBridge(bridge.clone());
        let output = match workflow.run_json(serde_json::to_string(&module_input)?, &mut host) {
            Ok(output) => output,
            Err(failure) => return encode(ProcessWorkflowResponse::failed(failure)),
        };
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
