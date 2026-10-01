use super::BoundModel;
use crate::core::model_call_scope::current_model_call_origin;
use crate::model_standard::CanonicalModelRequest;
use anyhow::Result;

impl BoundModel {
    pub(super) async fn apply_before_model(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelRequest> {
        if let Some(tools) = &self.tool_authority {
            tools.validate_model_tools(&request.tools, &self.hook_cwd)?;
        }
        let mut request = self.service.prepare_request(request)?;
        self.binding.bind_request(&mut request)?;
        let event = self
            .hooks
            .apply(crate::contracts::HookInput {
                event: crate::contracts::HookEvent::BeforeModel {
                    origin: current_model_call_origin(),
                    request,
                },
                attribution: self.hook_attribution,
                cwd: self.hook_cwd.clone(),
            })
            .await?;
        let crate::contracts::HookEvent::BeforeModel {
            request: transformed,
            ..
        } = event
        else {
            anyhow::bail!("hook changed model event kind");
        };
        let mut request = self.service.prepare_request(transformed)?;
        self.binding.bind_request(&mut request)?;
        if let Some(tools) = &self.tool_authority {
            tools.validate_model_tools(&request.tools, &self.hook_cwd)?;
        }
        Ok(request)
    }
}
