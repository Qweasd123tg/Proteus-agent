use std::collections::HashMap;

use anyhow::{Result, bail};
use proteus_contracts::process_module::{
    CompactorModuleObject, ContextBuilderModuleObject, ContextProviderModuleObject,
    MemoryModuleObject, ModuleRegistry, PatchModuleObject, PolicyModuleObject, ProcessModuleError,
    ProcessModuleResult, SearchModuleObject, ToolExposureModuleObject, ToolModuleObject,
    WorkflowModuleObject,
};
use serde_json::{Value, json};

type RegisterFn = fn(&mut dyn ModuleRegistry) -> ProcessModuleResult<()>;

pub struct CollectedModules {
    module_config: Value,
    pub tools: Vec<ToolModuleObject>,
    pub policies: HashMap<String, PolicyModuleObject>,
    pub patches: HashMap<String, PatchModuleObject>,
    pub searches: HashMap<String, SearchModuleObject>,
    pub memories: HashMap<String, MemoryModuleObject>,
    pub context_providers: HashMap<String, ContextProviderModuleObject>,
    pub contexts: HashMap<String, ContextBuilderModuleObject>,
    pub compactors: HashMap<String, CompactorModuleObject>,
    pub exposures: HashMap<String, ToolExposureModuleObject>,
    pub workflows: HashMap<String, WorkflowModuleObject>,
}

impl CollectedModules {
    pub fn load(slot: &str, module_id: &str, module_config: Value) -> Result<Self> {
        let mut modules = Self::with_config(module_config);
        if (slot, module_id) == ("tool", "reference.tools") {
            for register in [
                file_tools::register_modules as RegisterFn,
                git_tools::register_modules,
                shell_tool::register_modules,
                plan_tool::register_modules,
                rust_lsp::register_modules,
                skill_pack::register_modules,
                policy_pack::register_modules,
            ] {
                register(&mut modules).map_err(anyhow::Error::new)?;
            }
            return Ok(modules);
        }
        let register: RegisterFn = match (slot, module_id) {
            ("tool", "file_tools") => file_tools::register_modules,
            ("tool", "git_tools") => git_tools::register_modules,
            ("tool", "shell_tools") => shell_tool::register_modules,
            ("tool", "plan_tool") => plan_tool::register_modules,
            ("tool", "rust_lsp") => rust_lsp::register_modules,
            ("tool", "skill_tool") => skill_pack::register_modules,
            ("tool", "policy_tools") => policy_pack::register_modules,
            ("search", "rg") => rg_search::register_modules,
            ("patch", "direct") => direct_patch::register_modules,
            ("memory", "jsonl") => memory_pack::register_modules,
            ("memory", "sqlite") => sqlite_memory::register_modules,
            ("context", "simple" | "repo_aware" | "codex_context") => {
                context_pack::register_modules
            }
            ("context_provider", "skills") => skill_pack::register_modules,
            ("compactor", "codex") => codex_compactor::register_modules,
            ("tool_exposure", "codex_dynamic") => codex_tool_exposure::register_modules,
            ("policy", "allow_all" | "ask_write" | "codex_policy" | "opencode_policy") => {
                policy_pack::register_modules
            }
            (
                "workflow",
                "coding.single_loop" | "coding.codex_loop" | "coding.plan_execute_review",
            ) => coding_workflow::register_modules,
            _ => bail!("reference worker has no {slot} module {module_id:?}"),
        };

        register(&mut modules).map_err(anyhow::Error::new)?;
        Ok(modules)
    }

    fn with_config(module_config: Value) -> Self {
        Self {
            module_config,
            tools: Vec::new(),
            policies: HashMap::new(),
            patches: HashMap::new(),
            searches: HashMap::new(),
            memories: HashMap::new(),
            context_providers: HashMap::new(),
            contexts: HashMap::new(),
            compactors: HashMap::new(),
            exposures: HashMap::new(),
            workflows: HashMap::new(),
        }
    }
}

impl Default for CollectedModules {
    fn default() -> Self {
        Self::with_config(json!({}))
    }
}

fn insert<T>(map: &mut HashMap<String, T>, id: String, value: T) -> ProcessModuleResult<()> {
    if id.trim().is_empty() {
        return Err(ProcessModuleError::new("module id must not be empty"));
    }
    if map.insert(id.clone(), value).is_some() {
        return Err(ProcessModuleError::new(format!(
            "duplicate module id: {id}"
        )));
    }
    Ok(())
}

impl ModuleRegistry for CollectedModules {
    fn module_config(&self) -> &Value {
        &self.module_config
    }

    fn register_tool(&mut self, tool: ToolModuleObject) -> ProcessModuleResult<()> {
        self.tools.push(tool);
        Ok(())
    }

    fn register_policy(
        &mut self,
        module_id: String,
        policy: PolicyModuleObject,
    ) -> ProcessModuleResult<()> {
        insert(&mut self.policies, module_id, policy)
    }

    fn register_patch(
        &mut self,
        module_id: String,
        applier: PatchModuleObject,
    ) -> ProcessModuleResult<()> {
        insert(&mut self.patches, module_id, applier)
    }

    fn register_search(
        &mut self,
        module_id: String,
        backend: SearchModuleObject,
    ) -> ProcessModuleResult<()> {
        insert(&mut self.searches, module_id, backend)
    }

    fn register_memory(
        &mut self,
        module_id: String,
        store: MemoryModuleObject,
    ) -> ProcessModuleResult<()> {
        insert(&mut self.memories, module_id, store)
    }

    fn register_context_provider(
        &mut self,
        provider_id: String,
        provider: ContextProviderModuleObject,
    ) -> ProcessModuleResult<()> {
        insert(&mut self.context_providers, provider_id, provider)
    }

    fn register_context(
        &mut self,
        module_id: String,
        builder: ContextBuilderModuleObject,
    ) -> ProcessModuleResult<()> {
        insert(&mut self.contexts, module_id, builder)
    }

    fn register_compactor(
        &mut self,
        module_id: String,
        compactor: CompactorModuleObject,
    ) -> ProcessModuleResult<()> {
        insert(&mut self.compactors, module_id, compactor)
    }

    fn register_tool_exposure(
        &mut self,
        module_id: String,
        exposure: ToolExposureModuleObject,
    ) -> ProcessModuleResult<()> {
        insert(&mut self.exposures, module_id, exposure)
    }

    fn register_workflow(
        &mut self,
        module_id: String,
        workflow: WorkflowModuleObject,
    ) -> ProcessModuleResult<()> {
        insert(&mut self.workflows, module_id, workflow)
    }
}
