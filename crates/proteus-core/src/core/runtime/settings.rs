use super::*;

impl AgentRuntime {
    pub async fn model_catalog(&self) -> Result<Option<crate::contracts::ModelCatalog>> {
        self.snapshot().await.registry.model_catalog().await
    }

    pub async fn set_model_name(&self, model: String) -> Result<()> {
        let model = model.trim();
        anyhow::ensure!(!model.is_empty(), "model name must not be empty");
        let snapshot = self.snapshot().await;
        let catalog = snapshot.registry.model_catalog().await?;
        let selected = catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .models
                    .iter()
                    .find(|entry| entry.id == model)
                    .ok_or_else(|| {
                        anyhow::anyhow!("model {model:?} is absent from the provider catalog")
                    })
            })
            .transpose()?;
        let mut state = self.services.execution_state.write().await;
        anyhow::ensure!(
            state.runtime.epoch == snapshot.epoch,
            "model provider changed during catalog lookup; retry selection"
        );
        state.model_ref.model = model.to_owned();
        if let Some(selected) = selected {
            if state
                .reasoning
                .effort
                .as_ref()
                .is_none_or(|effort| !selected.reasoning_efforts.contains(effort))
            {
                state.reasoning.effort = selected.default_reasoning_effort.clone();
            }
            if state.reasoning.effort.as_deref() == Some("none")
                || selected.reasoning_efforts.is_empty()
            {
                state.reasoning.summary = false;
                state.reasoning.budget_tokens = None;
            }
        }
        Ok(())
    }

    /// Полная замена provider+model, например после смены `active_provider`
    /// через config builder: `reload_assembly` пересобирает model adapter, но
    /// не трогает runtime override model_ref.
    pub async fn set_model_ref(&self, model_ref: ModelRef) {
        self.services.execution_state.write().await.model_ref = model_ref;
    }

    pub async fn model_ref(&self) -> ModelRef {
        self.services.execution_state.read().await.model_ref.clone()
    }

    pub async fn set_reasoning_enabled(&self, enabled: bool) {
        let mut state = self.services.execution_state.write().await;
        let reasoning = &mut state.reasoning;
        if enabled {
            if reasoning.effort.is_none() || reasoning.effort.as_deref() == Some("none") {
                reasoning.effort = self.services.default_reasoning.effort.clone();
            }
            reasoning.summary = self.services.default_reasoning.summary;
            reasoning.budget_tokens = self.services.default_reasoning.budget_tokens;
        } else {
            reasoning.effort = None;
            reasoning.summary = false;
            reasoning.budget_tokens = None;
        }
    }

    pub async fn set_reasoning_effort(&self, effort: Option<String>) -> Result<()> {
        let snapshot = self.snapshot().await;
        let catalog = snapshot.registry.model_catalog().await?;
        let mut state = self.services.execution_state.write().await;
        anyhow::ensure!(
            state.runtime.epoch == snapshot.epoch,
            "model provider changed during catalog lookup; retry selection"
        );
        if let (Some(catalog), Some(effort)) = (&catalog, &effort) {
            let model = catalog
                .models
                .iter()
                .find(|entry| entry.id == state.model_ref.model)
                .ok_or_else(|| {
                    anyhow::anyhow!("active model is absent from the provider catalog")
                })?;
            anyhow::ensure!(
                model.reasoning_efforts.contains(effort),
                "unsupported reasoning effort {effort:?} for {}",
                model.id
            );
        }
        let reasoning = &mut state.reasoning;
        match effort.as_deref() {
            // «none» — первоклассное значение effort: выключает рассуждения
            // целиком. Веб-клиент шлёт его вместо пары /reasoning + /effort.
            Some("none") => {
                reasoning.effort = Some("none".to_owned());
                reasoning.summary = false;
                reasoning.budget_tokens = None;
            }
            // Конкретный effort включает рассуждения, даже если они были
            // выключены: summary/budget возвращаются к дефолтам конфига.
            Some(value) => {
                if !reasoning.is_enabled() {
                    reasoning.summary = self.services.default_reasoning.summary;
                    reasoning.budget_tokens = self.services.default_reasoning.budget_tokens;
                }
                reasoning.effort = Some(value.to_owned());
            }
            // null — «auto»: явного effort нет, остальное не трогаем.
            None => reasoning.effort = None,
        }
        Ok(())
    }

    pub async fn reasoning(&self) -> ReasoningConfig {
        self.services.execution_state.read().await.reasoning.clone()
    }
}
