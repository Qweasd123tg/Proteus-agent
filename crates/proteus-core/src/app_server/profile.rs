//! Profile publication shared by UI saves, explicit reload and file refresh.
use super::{AppServerEvent, AppServerHandle, prepare_assembly};
use crate::{
    core::{AppConfig, PreparedAssembly, RuntimeReloadReport, SessionConfigSnapshot},
    domain::PermissionMode,
};
use anyhow::Result;

impl AppServerHandle {
    pub(super) async fn publish_profile<F, Fut>(
        &self,
        config: AppConfig,
        assembly: PreparedAssembly,
        explicit_mode: Option<PermissionMode>,
        commit: F,
    ) -> Result<RuntimeReloadReport>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let previous = self.config.read().await.clone();
        let old_model = previous.selected_model_config()?;
        let next_model = config.selected_model_config()?;
        let provider_changed = config.active_provider != previous.active_provider;
        let model_changed = provider_changed
            || old_model.as_ref().map(|model| model.model_ref())
                != next_model.as_ref().map(|model| model.model_ref());
        let model_ref = if model_changed {
            next_model.as_ref().map(|model| model.model_ref())
        } else {
            None
        };
        let reasoning_changed = provider_changed
            || old_model.as_ref().map(|model| &model.reasoning)
                != next_model.as_ref().map(|model| &model.reasoning);
        let reasoning = if reasoning_changed {
            next_model.as_ref().map(|model| model.reasoning.clone())
        } else {
            None
        };
        let mode = explicit_mode.or_else(|| {
            (previous.permissions.mode != config.permissions.mode)
                .then_some(config.permissions.mode)
        });
        let snapshot = SessionConfigSnapshot::from_runtime_config(
            &config,
            assembly.registry(),
            config.permissions.mode,
        );
        let report = self
            .runtime
            .reload_assembly_with_commit(
                assembly,
                Some(snapshot),
                model_ref,
                mode,
                reasoning,
                commit,
            )
            .await?;
        *self.config.write().await = config;
        self.set_profile_error(None).await;
        let _ = self.events.send(AppServerEvent::ModulesReloaded {
            old_epoch: report.old_epoch,
            new_epoch: report.new_epoch,
            tool_names: report.tool_names.clone(),
        });
        Ok(report)
    }

    pub(super) async fn set_profile_error(&self, error: Option<String>) {
        let mut stored = self.profile_error.lock().await;
        if *stored != error {
            *stored = error.clone();
            let _ = self
                .events
                .send(AppServerEvent::ProfileReloadStatus { error });
        }
    }

    pub async fn reload_tools(&self) -> Result<RuntimeReloadReport> {
        let lock = self
            .config_path
            .as_deref()
            .map(super::config_builder::path_lock)
            .transpose()?;
        let _guard = match lock {
            Some(lock) => Some(lock.lock_owned().await),
            None => None,
        };
        let config = super::reload_tools_config(self.config_path.as_deref(), &self.config).await?;
        let assembly = prepare_assembly(&config, &self.cwd, self.config_path.as_deref()).await?;
        self.publish_profile(config, assembly, None, || async { Ok(()) })
            .await
    }
}
