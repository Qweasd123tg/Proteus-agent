use super::*;
use crate::{
    app_server::model_selection::selection_summary,
    contracts::ModelCatalog,
    core::AppConfig,
    domain::{ModelRef, PermissionMode, ReasoningConfig},
};
use serde_json::{Value, json};

fn rendered(active: &str, efforts: &[&str], error: bool) -> Value {
    let active = ModelRef::new("external", active);
    let reasoning = ReasoningConfig::default();
    let catalog: ModelCatalog = serde_json::from_value(json!({"models": [
        {"id":"first","display_name":"First","description":"Visible model","hidden":false,
         "reasoning_efforts": efforts, "default_reasoning_effort": null},
        {"id":"hidden","display_name":"Hidden","hidden":true,
         "reasoning_efforts":[], "default_reasoning_effort":null}
    ]}))
    .unwrap();
    let summary = selection_summary(
        &AppConfig::default(),
        &active,
        &reasoning,
        if error {
            Err(anyhow::anyhow!("catalog unavailable"))
        } else {
            Ok(Some(catalog))
        },
    );
    serde_json::to_value(render(
        ModelSelection {
            active,
            reasoning,
            summary,
        },
        input::modes(PermissionMode::Normal).unwrap(),
    ))
    .unwrap()
}

#[test]
fn provider_catalog_controls_visible_models_and_reasoning_without_guesses() {
    let options = rendered("first", &["high", "_default"], false);
    assert_eq!(
        options[1]["options"],
        json!([
            {"value":"first","name":"First","description":"Visible model"}
        ])
    );
    assert_eq!(options[2]["currentValue"], "_default");
    assert_eq!(options[2]["options"][1]["value"], "effort:high");
    assert_eq!(options[2]["options"][2]["value"], "effort:_default");
    assert_eq!(options[2]["options"].as_array().unwrap().len(), 3);
    let hidden = rendered("hidden", &[], false);
    assert_eq!(hidden[1]["options"].as_array().unwrap().len(), 2);
    assert_eq!(hidden[1]["currentValue"], "hidden");
    assert_eq!(
        hidden.as_array().unwrap().len(),
        2,
        "no unsupported reasoning selector"
    );
    assert_eq!(
        rendered("first", &[], true).as_array().unwrap().len(),
        1,
        "catalog errors must not create fallback models"
    );
}
