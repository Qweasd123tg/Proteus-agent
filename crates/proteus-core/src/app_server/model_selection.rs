use serde::Serialize;

use super::config_summary::{configured_model_options, configured_reasoning_effort_options};
use crate::{
    contracts::ModelCatalog,
    core::AppConfig,
    domain::{ModelRef, ReasoningConfig},
};

pub(super) struct SelectionSummary {
    pub models: Vec<ModelOption>,
    pub efforts: Vec<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub(super) struct ModelOption {
    pub provider: String,
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    pub hidden: bool,
    pub reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}

pub(super) struct ModelSelection {
    pub active: ModelRef,
    pub reasoning: ReasoningConfig,
    pub summary: SelectionSummary,
}

impl super::AppServerHandle {
    pub(super) async fn model_selection(&self) -> ModelSelection {
        let active = self.runtime.model_ref().await;
        let reasoning = self.runtime.reasoning().await;
        let config = self.config.read().await.clone();
        let summary = selection_summary(
            &config,
            &active,
            &reasoning,
            self.runtime.model_catalog().await,
        );
        ModelSelection {
            active,
            reasoning,
            summary,
        }
    }
}

pub(super) fn selection_summary(
    config: &AppConfig,
    active: &ModelRef,
    reasoning: &ReasoningConfig,
    catalog: anyhow::Result<Option<ModelCatalog>>,
) -> SelectionSummary {
    match catalog {
        Ok(Some(catalog)) => SelectionSummary {
            efforts: catalog
                .models
                .iter()
                .find(|model| model.id == active.model)
                .map(|model| model.reasoning_efforts.clone())
                .unwrap_or_default(),
            models: catalog
                .models
                .into_iter()
                .map(|model| ModelOption {
                    provider: active.provider.clone(),
                    name: model.id,
                    label: model.display_name,
                    description: model.description,
                    hidden: model.hidden,
                    reasoning_efforts: model.reasoning_efforts,
                    default_reasoning_effort: model.default_reasoning_effort,
                })
                .collect(),
            error: None,
        },
        Ok(None) => {
            let mut efforts = configured_reasoning_effort_options(config, active, reasoning);
            if !efforts.iter().any(|value| value == "none") {
                efforts.insert(0, "none".into());
            }
            SelectionSummary {
                models: configured_model_options(config)
                    .into_iter()
                    .filter(|model| model.provider == active.provider)
                    .map(|model| ModelOption {
                        label: format!("{}/{}", model.provider, model.model),
                        provider: model.provider,
                        name: model.model,
                        description: None,
                        hidden: false,
                        reasoning_efforts: Vec::new(),
                        default_reasoning_effort: None,
                    })
                    .collect(),
                efforts,
                error: None,
            }
        }
        Err(error) => SelectionSummary {
            models: Vec::new(),
            efforts: Vec::new(),
            error: Some(format!("{error:#}")),
        },
    }
}
