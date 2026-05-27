//! Adapter: `ToolExposureObject` -> `Arc<dyn ToolExposure>`.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{PluginToolExposure_TO, ToolExposureObject},
};

use crate::contracts::{ToolExposure, ToolExposureInput, ToolExposureOutput};

pub struct PluginToolExposureAdapter {
    inner: Arc<ToolExposureObject>,
}

impl PluginToolExposureAdapter {
    pub fn new(inner: ToolExposureObject) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }
}

#[async_trait]
impl ToolExposure for PluginToolExposureAdapter {
    async fn select(&self, input: ToolExposureInput) -> Result<ToolExposureOutput> {
        let input_json = serde_json::to_string(&input)
            .with_context(|| "plugin tool exposure: serialize ToolExposureInput failed")?;
        let inner = self.inner.clone();
        let output_json = tokio::task::spawn_blocking(move || {
            match PluginToolExposure_TO::select_json(&*inner, RString::from(input_json)) {
                RResult::ROk(output) => Ok(output.into_string()),
                RResult::RErr(error) => {
                    Err(anyhow!("plugin tool exposure error: {}", error.message))
                }
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin tool exposure join error: {join_err}"))??;

        serde_json::from_str(&output_json)
            .with_context(|| "plugin tool exposure returned invalid ToolExposureOutput JSON")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::{
        abi_stable::{sabi_trait::TD_Opaque, std_types::RResult::ROk},
        domain::{AgentTask, ToolSafety, ToolSpec},
        plugin::{PluginToolExposure, PluginToolExposure_TO, PluginToolExposureError},
    };
    use serde_json::json;

    struct FirstToolExposure;
    impl PluginToolExposure for FirstToolExposure {
        fn select_json(&self, input_json: RString) -> RResult<RString, PluginToolExposureError> {
            let mut input: ToolExposureInput =
                serde_json::from_str(input_json.as_str()).expect("tool exposure input");
            input.candidates.truncate(1);
            let output = ToolExposureOutput::new(input.candidates);
            ROk(serde_json::to_string(&output).unwrap().into())
        }
    }

    struct FailToolExposure;
    impl PluginToolExposure for FailToolExposure {
        fn select_json(&self, _input_json: RString) -> RResult<RString, PluginToolExposureError> {
            RResult::RErr(PluginToolExposureError::new("selection failed"))
        }
    }

    struct BrokenJsonToolExposure;
    impl PluginToolExposure for BrokenJsonToolExposure {
        fn select_json(&self, _input_json: RString) -> RResult<RString, PluginToolExposureError> {
            ROk(RString::from("not json"))
        }
    }

    fn wrap(exposure: impl PluginToolExposure + 'static) -> PluginToolExposureAdapter {
        let obj = PluginToolExposure_TO::from_value(exposure, TD_Opaque);
        PluginToolExposureAdapter::new(obj)
    }

    fn make_input() -> ToolExposureInput {
        ToolExposureInput::new(
            crate::contracts::ToolExposureRequest::new(AgentTask::new(
                "edit file",
                std::path::PathBuf::from("/tmp"),
            )),
            vec![
                ToolSpec::new("read_file", "read", json!({}), ToolSafety::ReadOnly),
                ToolSpec::new("write_file", "write", json!({}), ToolSafety::WritesFiles),
            ],
        )
    }

    #[tokio::test]
    async fn plugin_success_round_trip() {
        let adapter = wrap(FirstToolExposure);
        let output = adapter.select(make_input()).await.unwrap();
        assert_eq!(output.tools.len(), 1);
        assert_eq!(output.tools[0].name, "read_file");
    }

    #[tokio::test]
    async fn plugin_rerror_propagates_as_anyhow() {
        let adapter = wrap(FailToolExposure);
        let err = adapter.select(make_input()).await.unwrap_err();
        assert!(err.to_string().contains("selection failed"), "{err}");
    }

    #[tokio::test]
    async fn invalid_json_propagates_as_anyhow() {
        let adapter = wrap(BrokenJsonToolExposure);
        let err = adapter.select(make_input()).await.unwrap_err();
        assert!(
            err.to_string().contains("invalid ToolExposureOutput"),
            "{err}"
        );
    }
}
