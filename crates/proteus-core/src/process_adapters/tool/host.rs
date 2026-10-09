use super::super::host_rpc::{callback_error, decode, encode};
use crate::{
    contracts::{
        CancellationToken, ConversationReader, ReadConversationRequest,
        TOOL_HOST_READ_CONVERSATION_METHOD,
    },
    domain::ToolCall,
};
use proteus_module_protocol::{
    ProcessModuleRpcError,
    v3::{AsyncHostRequestDispatcher, ComponentHostRequest, HostRequestFuture},
};
use std::sync::Arc;

pub(super) struct ToolHost {
    pub conversation: Option<Arc<dyn ConversationReader>>,
    pub call: ToolCall,
    pub cancellation: CancellationToken,
}

impl AsyncHostRequestDispatcher for ToolHost {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        let reader = self.conversation.clone();
        let call = self.call.clone();
        let cancellation = self.cancellation.clone();
        Box::pin(async move {
            let method = request.method;
            if method != TOOL_HOST_READ_CONVERSATION_METHOD {
                return Err(ProcessModuleRpcError::new(
                    -32601,
                    "unsupported tool host method",
                ));
            }
            let _: ReadConversationRequest = decode(request.params, &method)?;
            let result = async {
                anyhow::ensure!(!cancellation.is_cancelled(), "tool invocation canceled");
                let reader =
                    reader.ok_or_else(|| anyhow::anyhow!("tool invocation has no conversation"))?;
                let snapshot = reader.read().await?.for_tool(&call)?;
                anyhow::ensure!(!cancellation.is_cancelled(), "tool invocation canceled");
                Ok(snapshot)
            }
            .await;
            encode(
                result.map_err(|error| callback_error(&method, &error))?,
                &method,
            )
        })
    }
}
