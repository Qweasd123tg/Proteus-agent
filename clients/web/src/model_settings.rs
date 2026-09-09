//! Apply server-owned model selection and its matching reasoning options together.
use leptos::prelude::*;
use serde_json::Value;

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
    pub fn apply(self, config: &Value) {
        if let Some(model) = config.pointer("/model/name").and_then(Value::as_str) {
            self.model.set(model.to_owned());
        }
        let options = config
            .get("model_options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| {
                Some(ModelOption {
                    name: value.get("name")?.as_str()?.to_owned(),
                    label: value.get("label")?.as_str()?.to_owned(),
                    hidden: value
                        .get("hidden")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect();
        self.models.set(options);
        let enabled = config
            .pointer("/reasoning/enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.enabled.set(enabled);
        let effort = config
            .pointer("/reasoning/effort")
            .and_then(Value::as_str)
            .map(ReasoningEffort::from_value)
            .unwrap_or(if enabled {
                ReasoningEffort::Config
            } else {
                ReasoningEffort::None
            });
        self.effort.set(effort);
        self.efforts.set(
            config
                .pointer("/reasoning/effort_options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        );
        if let Some(error) = config.get("model_catalog_error").and_then(Value::as_str) {
            self.status.set(TransportStatus::Error(format!(
                "Не удалось загрузить каталог моделей: {error}"
            )));
        }
    }
}
