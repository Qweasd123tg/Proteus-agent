use super::*;

impl AgentRuntime {
    pub async fn reload_assembly(
        &self,
        assembly: PreparedAssembly,
        config_snapshot: Option<SessionConfigSnapshot>,
    ) -> Result<RuntimeReloadReport> {
        self.reload_assembly_with_effective_settings(assembly, config_snapshot, None, None)
            .await
    }

    pub(crate) async fn reload_assembly_with_effective_settings(
        &self,
        assembly: PreparedAssembly,
        config_snapshot: Option<SessionConfigSnapshot>,
        model_ref: Option<ModelRef>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<RuntimeReloadReport> {
        self.reload_assembly_with_commit(
            assembly,
            config_snapshot,
            model_ref,
            permission_mode,
            None,
            || async { Ok(()) },
        )
        .await
    }

    pub(crate) async fn reload_assembly_with_commit<F, Fut>(
        &self,
        assembly: PreparedAssembly,
        config_snapshot: Option<SessionConfigSnapshot>,
        model_ref: Option<ModelRef>,
        permission_mode: Option<PermissionMode>,
        reasoning: Option<ReasoningConfig>,
        commit: F,
    ) -> Result<RuntimeReloadReport>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        if self.session.session_store.is_some() && config_snapshot.is_none() {
            anyhow::bail!("persisted runtime reload requires a config snapshot");
        }
        let _reload_guard = self.services.reload_lock.lock().await;
        let snapshot = self.capture_execution_snapshot().await;
        let old_epoch = snapshot.runtime.epoch;
        let new_epoch = old_epoch.next();
        let model_ref = if assembly.registry().model_config.is_none() {
            None
        } else {
            model_ref
                .or_else(|| snapshot.model_ref.clone())
                .or_else(|| {
                    assembly
                        .registry()
                        .model_config
                        .as_ref()
                        .map(|cfg| cfg.model_ref())
                })
        };
        let mut runtime = RuntimeSnapshot::new(new_epoch, assembly, config_snapshot);
        runtime.registry.tools = runtime.registry.tools_for_model(model_ref.as_ref())?;
        let tool_names = runtime
            .registry
            .tools
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        // Everything that may fail is prepared before persistence. All settings
        // writers share reload_lock; publication after commit is infallible.
        commit().await?;
        let mut state = self.services.execution_state.write().await;
        state.runtime = runtime;
        state.model_ref = model_ref;
        if let Some(permission_mode) = permission_mode {
            state.permission_mode = permission_mode;
        }
        if let Some(reasoning) = reasoning {
            state.reasoning = reasoning;
        }
        Ok(RuntimeReloadReport {
            old_epoch: old_epoch.as_u64(),
            new_epoch: new_epoch.as_u64(),
            tool_names,
        })
    }
}
