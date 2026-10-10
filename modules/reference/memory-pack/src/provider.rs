use proteus_contracts::{
    contracts::ProcessContextProviderRequest,
    domain::{ContextChunk, MemoryQuery},
    process_module::{ContextProviderModule, ProcessModuleError, ProcessModuleResult},
};
use std::sync::Arc;

pub(crate) struct MemoryProvider {
    pub store: Arc<super::JsonlMemoryStoreModule>,
}

impl ContextProviderModule for MemoryProvider {
    fn provide_json(&self, input_json: String) -> ProcessModuleResult<String> {
        let request: ProcessContextProviderRequest = serde_json::from_str(&input_json)
            .map_err(|error| ProcessModuleError::new(error.to_string()))?;
        let query: MemoryQuery =
            serde_json::from_value(request.input.metadata).map_err(|error| {
                ProcessModuleError::new(format!("invalid memory provider query: {error}"))
            })?;
        let chunks = self
            .store
            .recall(&query)?
            .into_iter()
            .map(|item| {
                ContextChunk::new(format!("memory:{}", item.kind), item.content)
                    .with_metadata(item.metadata)
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&chunks).map_err(|error| ProcessModuleError::new(error.to_string()))
    }
}
