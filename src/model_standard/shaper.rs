use anyhow::Result;

use crate::model_standard::CanonicalModelRequest;

#[derive(Debug, Default, Clone)]
pub struct RequestShaper;

impl RequestShaper {
    pub fn shape(&self, request: CanonicalModelRequest) -> Result<CanonicalModelRequest> {
        Ok(request)
    }
}
