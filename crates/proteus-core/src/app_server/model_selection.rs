use serde_json::{Value, json};

use super::config_summary::{configured_model_options, configured_reasoning_effort_options};
use crate::{
    contracts::ModelCatalog,
    core::AppConfig,
    domain::{ModelRef, ReasoningConfig},
};

pub(super) struct SelectionSummary {
    pub models: Vec<Value>,
    pub efforts: Vec<String>,
    pub error: Option<String>,
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
                .map(|model| {
                    json!({
                        "provider": active.provider,
                        "name": model.id,
                        "label": model.display_name,
                        "description": model.description,
                        "hidden": model.hidden,
                        "reasoning_efforts": model.reasoning_efforts,
                        "default_reasoning_effort": model.default_reasoning_effort,
                    })
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
                    .map(|model| {
                        json!({ "provider": model.provider, "name": model.model,
                        "label": format!("{}/{}", model.provider, model.model) })
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
