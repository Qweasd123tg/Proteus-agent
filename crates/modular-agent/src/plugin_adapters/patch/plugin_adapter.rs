//! Адаптер: `PatchApplierObject` → `Arc<dyn PatchApplier>`.
//!
//! `PatchApplier` в ядре async, `PluginPatchApplier` — sync (sabi_trait
//! не поддерживает async). Мост через `tokio::task::spawn_blocking`, DTO
//! через JSON. Эталон — `plugin_adapters/tool.rs`.

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;

use agent_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{PatchApplierObject, PluginPatchApplier_TO},
};

use crate::{
    contracts::PatchApplier,
    domain::{Patch, PatchResult},
};

pub struct PluginPatchAdapter {
    inner: Arc<PatchApplierObject>,
    cwd: PathBuf,
}

impl PluginPatchAdapter {
    pub fn new(inner: Arc<PatchApplierObject>, cwd: PathBuf) -> Self {
        Self { inner, cwd }
    }
}

#[async_trait]
impl PatchApplier for PluginPatchAdapter {
    async fn apply(&self, patch: Patch) -> Result<PatchResult> {
        let patch_json = serde_json::to_string(&patch)
            .with_context(|| "plugin patch: serialize Patch failed")?;
        let cwd_string = self.cwd.to_string_lossy().into_owned();
        let inner = self.inner.clone();

        let result_json = tokio::task::spawn_blocking(move || {
            let patch_r = RString::from(patch_json);
            let cwd_r = RString::from(cwd_string);
            let outcome = PluginPatchApplier_TO::apply_json(&*inner, patch_r, cwd_r);
            match outcome {
                RResult::ROk(s) => Ok(s.into_string()),
                RResult::RErr(err) => Err(anyhow!("plugin patch error: {}", err.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin patch join error: {join_err}"))??;

        let result: PatchResult = serde_json::from_str(&result_json)
            .with_context(|| "plugin patch returned invalid result JSON")?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_contracts::{
        abi_stable::{sabi_trait::TD_Opaque, std_types::RResult::ROk},
        plugin::{PluginPatchApplier, PluginPatchApplier_TO, PluginPatchError},
    };

    struct NoopPatch;
    impl PluginPatchApplier for NoopPatch {
        fn apply_json(&self, _patch: RString, _cwd: RString) -> RResult<RString, PluginPatchError> {
            let result = PatchResult::new(true, "noop");
            ROk(serde_json::to_string(&result).unwrap().into())
        }
    }

    struct FailPatch;
    impl PluginPatchApplier for FailPatch {
        fn apply_json(&self, _patch: RString, _cwd: RString) -> RResult<RString, PluginPatchError> {
            RResult::RErr(PluginPatchError::new("plugin exploded"))
        }
    }

    struct BrokenJsonPatch;
    impl PluginPatchApplier for BrokenJsonPatch {
        fn apply_json(&self, _patch: RString, _cwd: RString) -> RResult<RString, PluginPatchError> {
            ROk(RString::from("not json"))
        }
    }

    fn wrap(applier: impl PluginPatchApplier + 'static) -> PluginPatchAdapter {
        let obj = PluginPatchApplier_TO::from_value(applier, TD_Opaque);
        PluginPatchAdapter::new(Arc::new(obj), PathBuf::from("/tmp"))
    }

    #[tokio::test]
    async fn plugin_success_round_trip() {
        let adapter = wrap(NoopPatch);
        let result = adapter.apply(Patch::new("dummy")).await.unwrap();
        assert!(result.ok);
        assert_eq!(result.summary, "noop");
    }

    #[tokio::test]
    async fn plugin_rerror_propagates_as_anyhow() {
        let adapter = wrap(FailPatch);
        let err = adapter.apply(Patch::new("x")).await.unwrap_err();
        assert!(err.to_string().contains("plugin exploded"), "{err}");
    }

    #[tokio::test]
    async fn invalid_json_propagates_as_anyhow() {
        let adapter = wrap(BrokenJsonPatch);
        let err = adapter.apply(Patch::new("x")).await.unwrap_err();
        assert!(err.to_string().contains("invalid result JSON"), "{err}");
    }
}
