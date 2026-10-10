use proteus_contracts::{
    contracts::ProcessContextProviderRequest,
    domain::SearchQuery,
    process_module::{ContextProviderModule, ProcessModuleError, ProcessModuleResult},
};

pub(crate) struct SearchProvider;

impl ContextProviderModule for SearchProvider {
    fn provide_json(&self, input_json: String) -> ProcessModuleResult<String> {
        let request: ProcessContextProviderRequest = serde_json::from_str(&input_json)
            .map_err(|error| ProcessModuleError::new(error.to_string()))?;
        let query: SearchQuery =
            serde_json::from_value(request.input.metadata).map_err(|error| {
                ProcessModuleError::new(format!("invalid search provider query: {error}"))
            })?;
        let chunks = super::run_rg(query).map_err(ProcessModuleError::new)?;
        serde_json::to_string(&chunks).map_err(|error| ProcessModuleError::new(error.to_string()))
    }
}
