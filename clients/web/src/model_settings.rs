//! Apply server-owned model selection and its matching reasoning options together.
use leptos::prelude::*;
use proteus_contracts::app_protocol::config::ConfigSummary;

use crate::types::{ModelOption, ReasoningEffort, TransportStatus};

#[derive(Clone, Copy)]
pub(crate) struct ModelSettings {
    pub model: WriteSignal<String>,
    pub models: WriteSignal<Vec<ModelOption>>,
    pub enabled: WriteSignal<bool>,
    pub effort: WriteSignal<ReasoningEffort>,
    pub efforts: WriteSignal<Vec<String>>,
    pub status: WriteSignal<TransportStatus>,
}

impl ModelSettings {
    pub fn apply(self, config: &ConfigSummary) {
        self.model.set(config.model.name.clone());
        self.models.set(
            config
                .model_options
                .iter()
                .map(|model| ModelOption {
                    name: model.name.clone(),
                    label: model.label.clone(),
                    hidden: model.hidden,
                })
                .collect(),
        );
        self.enabled.set(config.reasoning.enabled);
        self.effort.set(
            config
                .reasoning
                .effort
                .as_deref()
                .map(ReasoningEffort::from_value)
                .unwrap_or(if config.reasoning.enabled {
                    ReasoningEffort::Config
                } else {
                    ReasoningEffort::None
                }),
        );
        self.efforts.set(config.reasoning.effort_options.clone());
        if let Some(error) = &config.model_catalog_error {
            self.status.set(TransportStatus::Error(format!(
                "Не удалось загрузить каталог моделей: {error}"
            )));
        }
    }
}
