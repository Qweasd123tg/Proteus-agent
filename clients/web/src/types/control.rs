pub(crate) use proteus_contracts::{
    app_protocol::{
        AppApprovalPreview as ApprovalPreviewInfo, AppApprovalRequest as ApprovalRequestInfo,
        AppPendingRequests as PendingControlPlaneInfo, AppQueuedUserMessage as QueuedPromptInfo,
    },
    contracts::{ApprovalCacheScope, UserInputRequest as UserInputRequestInfo},
};
pub(crate) trait ApprovalCacheLabel {
    fn label(self) -> &'static str;
}
impl ApprovalCacheLabel for ApprovalCacheScope {
    fn label(self) -> &'static str {
        match self {
            Self::None => "Один раз",
            Self::ExactCall => "Точно",
            Self::ExactCommand => "Команда",
            Self::WorkspaceWrite => "Workspace",
        }
    }
}
