//! Адаптер: `MemoryStoreObject` → `Arc<dyn MemoryStore>`.
//!
//! `MemoryStore` в ядре async (`remember`/`recall`), `PluginMemoryStore` —
//! sync (sabi_trait не поддерживает async). Мост через
//! `tokio::task::spawn_blocking`, DTO через JSON. Эталон —
//! `plugin_adapters/search/plugin_adapter.rs`.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;

use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{
        MemoryPolicyObject, MemoryStoreObject, PluginMemoryPolicy_TO, PluginMemoryPolicyInput,
        PluginMemoryStore_TO,
    },
};

use crate::{
    contracts::{MemoryPolicy, MemoryPolicyInput, MemoryPolicyOutput, MemoryStore},
    domain::{MemoryItem, MemoryOp, MemoryPolicyPlan, MemoryQuery},
};

pub struct PluginMemoryAdapter {
    inner: Arc<MemoryStoreObject>,
}

impl PluginMemoryAdapter {
    pub fn new(store: MemoryStoreObject) -> Self {
        Self {
            inner: Arc::new(store),
        }
    }
}

#[async_trait]
impl MemoryStore for PluginMemoryAdapter {
    async fn remember(&self, item: MemoryItem) -> Result<()> {
        let item_json = serde_json::to_string(&item)
            .with_context(|| "plugin memory: serialize MemoryItem failed")?;
        let inner = self.inner.clone();

        tokio::task::spawn_blocking(move || {
            let payload = RString::from(item_json);
            match PluginMemoryStore_TO::remember_json(&*inner, payload) {
                RResult::ROk(()) => Ok(()),
                RResult::RErr(err) => Err(anyhow!("plugin memory error: {}", err.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin memory join error: {join_err}"))?
    }

    async fn recall(&self, query: MemoryQuery) -> Result<Vec<MemoryItem>> {
        let query_json = serde_json::to_string(&query)
            .with_context(|| "plugin memory: serialize MemoryQuery failed")?;
        let inner = self.inner.clone();

        let result_json = tokio::task::spawn_blocking(move || {
            let q = RString::from(query_json);
            match PluginMemoryStore_TO::recall_json(&*inner, q) {
                RResult::ROk(s) => Ok(s.into_string()),
                RResult::RErr(err) => Err(anyhow!("plugin memory error: {}", err.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin memory join error: {join_err}"))??;

        let items: Vec<MemoryItem> = serde_json::from_str(&result_json)
            .with_context(|| "plugin memory returned invalid result JSON")?;
        Ok(items)
    }
}

pub struct PluginMemoryPolicyAdapter {
    inner: Arc<MemoryPolicyObject>,
}

impl PluginMemoryPolicyAdapter {
    pub fn new(policy: MemoryPolicyObject) -> Self {
        Self {
            inner: Arc::new(policy),
        }
    }
}

#[async_trait]
impl MemoryPolicy for PluginMemoryPolicyAdapter {
    async fn after_turn(
        &self,
        input: MemoryPolicyInput<'_>,
        memory: &dyn MemoryStore,
    ) -> Result<MemoryPolicyOutput> {
        let dto = PluginMemoryPolicyInput {
            task: input.task.clone(),
            output: input.output.clone(),
            new_messages: input.new_messages.to_vec(),
        };
        let input_json = serde_json::to_string(&dto)
            .with_context(|| "plugin memory policy: serialize input failed")?;
        let inner = self.inner.clone();

        let result_json = tokio::task::spawn_blocking(move || {
            match PluginMemoryPolicy_TO::after_turn_json(&*inner, RString::from(input_json)) {
                RResult::ROk(s) => Ok(s.into_string()),
                RResult::RErr(err) => Err(anyhow!("plugin memory policy error: {}", err.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin memory policy join error: {join_err}"))??;

        let plan: MemoryPolicyPlan = serde_json::from_str(&result_json)
            .with_context(|| "plugin memory policy returned invalid plan JSON")?;
        apply_memory_plan(plan, memory).await
    }
}

async fn apply_memory_plan(
    plan: MemoryPolicyPlan,
    memory: &dyn MemoryStore,
) -> Result<MemoryPolicyOutput> {
    let mut written_kinds = Vec::new();
    for op in plan.ops {
        if let MemoryOp::Remember { item } = op {
            let kind = item.kind.clone();
            memory.remember(item).await?;
            written_kinds.push(kind);
        }
    }
    Ok(MemoryPolicyOutput { written_kinds })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use proteus_contracts::{
        abi_stable::{sabi_trait::TD_Opaque, std_types::RResult::ROk},
        plugin::{
            PluginMemoryError, PluginMemoryPolicy, PluginMemoryPolicy_TO, PluginMemoryPolicyError,
            PluginMemoryStore, PluginMemoryStore_TO,
        },
    };
    use tokio::sync::Mutex;

    use crate::{
        domain::{AgentOutput, AgentTask},
        model_standard::{CanonicalMessage, MessageRole},
    };

    struct InMemoryStore;
    impl PluginMemoryStore for InMemoryStore {
        fn remember_json(&self, _item: RString) -> RResult<(), PluginMemoryError> {
            ROk(())
        }
        fn recall_json(&self, _query: RString) -> RResult<RString, PluginMemoryError> {
            let items = vec![MemoryItem::new(
                "preference",
                "likes tabs",
                serde_json::Value::Null,
            )];
            ROk(serde_json::to_string(&items).unwrap().into())
        }
    }

    struct FailStore;
    impl PluginMemoryStore for FailStore {
        fn remember_json(&self, _item: RString) -> RResult<(), PluginMemoryError> {
            RResult::RErr(PluginMemoryError::new("write exploded"))
        }
        fn recall_json(&self, _query: RString) -> RResult<RString, PluginMemoryError> {
            RResult::RErr(PluginMemoryError::new("read exploded"))
        }
    }

    struct BrokenJsonStore;
    impl PluginMemoryStore for BrokenJsonStore {
        fn remember_json(&self, _item: RString) -> RResult<(), PluginMemoryError> {
            ROk(())
        }
        fn recall_json(&self, _query: RString) -> RResult<RString, PluginMemoryError> {
            ROk(RString::from("not json"))
        }
    }

    fn wrap(store: impl PluginMemoryStore + 'static) -> PluginMemoryAdapter {
        let obj = PluginMemoryStore_TO::from_value(store, TD_Opaque);
        PluginMemoryAdapter::new(obj)
    }

    fn make_item() -> MemoryItem {
        MemoryItem::new("preference", "dark mode", serde_json::Value::Null)
    }

    fn make_query() -> MemoryQuery {
        MemoryQuery::new("tabs", 10)
    }

    #[tokio::test]
    async fn plugin_remember_success() {
        let adapter = wrap(InMemoryStore);
        adapter.remember(make_item()).await.unwrap();
    }

    #[tokio::test]
    async fn plugin_recall_success() {
        let adapter = wrap(InMemoryStore);
        let items = adapter.recall(make_query()).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "preference");
        assert_eq!(items[0].content, "likes tabs");
    }

    #[tokio::test]
    async fn plugin_remember_rerror_propagates() {
        let adapter = wrap(FailStore);
        let err = adapter.remember(make_item()).await.unwrap_err();
        assert!(err.to_string().contains("write exploded"), "{err}");
    }

    #[tokio::test]
    async fn plugin_recall_rerror_propagates() {
        let adapter = wrap(FailStore);
        let err = adapter.recall(make_query()).await.unwrap_err();
        assert!(err.to_string().contains("read exploded"), "{err}");
    }

    #[tokio::test]
    async fn plugin_recall_invalid_json_errors() {
        let adapter = wrap(BrokenJsonStore);
        let err = adapter.recall(make_query()).await.unwrap_err();
        assert!(err.to_string().contains("invalid result JSON"), "{err}");
    }

    struct RememberingPolicy;

    impl PluginMemoryPolicy for RememberingPolicy {
        fn after_turn_json(
            &self,
            _input_json: RString,
        ) -> RResult<RString, PluginMemoryPolicyError> {
            let plan = MemoryPolicyPlan::new(vec![MemoryOp::Remember {
                item: MemoryItem::new("plugin:note", "remembered", serde_json::Value::Null),
            }]);
            ROk(serde_json::to_string(&plan).unwrap().into())
        }
    }

    #[derive(Default)]
    struct RecordingMemory {
        items: Mutex<Vec<MemoryItem>>,
    }

    #[async_trait]
    impl MemoryStore for RecordingMemory {
        async fn remember(&self, item: MemoryItem) -> Result<()> {
            self.items.lock().await.push(item);
            Ok(())
        }

        async fn recall(&self, _query: MemoryQuery) -> Result<Vec<MemoryItem>> {
            Ok(self.items.lock().await.clone())
        }
    }

    #[tokio::test]
    async fn plugin_memory_policy_applies_declarative_ops() {
        let obj = PluginMemoryPolicy_TO::from_value(RememberingPolicy, TD_Opaque);
        let adapter = PluginMemoryPolicyAdapter::new(obj);
        let memory = RecordingMemory::default();
        let task = AgentTask::new("task", std::path::PathBuf::from("/tmp"));
        let output = AgentOutput::text("done");
        let messages = vec![CanonicalMessage::text(MessageRole::Assistant, "done")];

        let result = adapter
            .after_turn(
                MemoryPolicyInput {
                    task: &task,
                    output: &output,
                    new_messages: &messages,
                },
                &memory,
            )
            .await
            .unwrap();

        assert_eq!(result.written_kinds, ["plugin:note"]);
        let items = memory.items.lock().await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "remembered");
    }
}
