//! Adapter: `SubagentObject` -> `Arc<dyn SubagentRunner>`.
//!
//! Subagent plugins are sync ABI objects. `run` executes them in
//! `spawn_blocking`; async runtime operations (model, tools, exposure,
//! events) go through the same narrow `PluginWorkflowHost` bridge that
//! workflow plugins use (see `plugin_adapters/workflow`).

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    plugin::{PluginSubagent_TO, PluginWorkflowHost_TO, PluginWorkflowHostMut, SubagentObject},
};
use tokio::runtime::Handle;

use crate::{
    contracts::{
        RuntimeContext, SubagentRequest, SubagentResult, SubagentRoleSpec, SubagentRunner,
    },
    plugin_adapters::workflow::WorkflowHost,
};

pub struct PluginSubagentAdapter {
    inner: Arc<SubagentObject>,
}

impl PluginSubagentAdapter {
    pub fn new(inner: SubagentObject) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }
}

#[async_trait]
impl SubagentRunner for PluginSubagentAdapter {
    fn roles(&self) -> Vec<SubagentRoleSpec> {
        // Контракт roles() не возвращает Result: ошибка плагина здесь
        // трактуется как "ролей нет" (делегирование выключено), а причина
        // логируется, чтобы не терять диагностику.
        let roles_json = match PluginSubagent_TO::roles_json(&*self.inner) {
            RResult::ROk(json) => json.into_string(),
            RResult::RErr(error) => {
                eprintln!(
                    "warning: subagent plugin roles_json failed: {}",
                    error.message
                );
                return Vec::new();
            }
        };
        match serde_json::from_str(&roles_json) {
            Ok(roles) => roles,
            Err(error) => {
                eprintln!(
                    "warning: subagent plugin returned invalid Vec<SubagentRoleSpec> JSON: {error}"
                );
                Vec::new()
            }
        }
    }

    fn supports_collaboration(&self) -> bool {
        false
    }

    fn supports_collaboration_messages(&self) -> bool {
        false
    }

    async fn run(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentResult> {
        let request_json = serde_json::to_string(&request)
            .with_context(|| "subagent plugin: serialize SubagentRequest failed")?;
        let inner = self.inner.clone();
        let handle = Handle::current();
        let host_ctx = ctx.clone();

        let result_json = tokio::task::spawn_blocking(move || {
            let mut host = WorkflowHost::new(host_ctx, handle);
            let mut host_to: PluginWorkflowHostMut<'_> =
                PluginWorkflowHost_TO::from_ptr(&mut host, TD_Opaque);
            match PluginSubagent_TO::run_json(&*inner, RString::from(request_json), &mut host_to) {
                RResult::ROk(output) => Ok(output.into_string()),
                RResult::RErr(error) => Err(anyhow!("subagent plugin error: {}", error.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("subagent plugin join error: {join_err}"))??;

        serde_json::from_str(&result_json)
            .with_context(|| "subagent plugin returned invalid SubagentResult JSON")
    }
}
