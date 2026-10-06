pub(crate) use proteus_contracts::{
    app_protocol::{
        AppApprovalPreview as ApprovalPreviewInfo, AppApprovalRequest as ApprovalRequestInfo,
        AppPendingRequests as PendingControlPlaneInfo, AppQueuedUserMessage as QueuedPromptInfo,
    },
    contracts::{ApprovalCacheScope, UserInputRequest as UserInputRequestInfo},
};
pub(crate) trait ApprovalCacheLabel {
    fn button_label(self) -> &'static str;
    fn description(self) -> &'static str;
}
impl ApprovalCacheLabel for ApprovalCacheScope {
    fn button_label(self) -> &'static str {
        match self {
            Self::None => "Разрешить",
            Self::ExactCall => "Разрешать этот вызов",
            Self::ExactCommand => "Разрешать эту команду",
            Self::WorkspaceWrite => "Разрешать запись в проекте",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::None => "Только этот вызов. Следующий запрос снова потребует разрешения.",
            Self::ExactCall => {
                "Для этого агента до перезапуска сессии: тот же инструмент с теми же параметрами в этом каталоге."
            }
            Self::ExactCommand => {
                "Для этого агента до перезапуска сессии: та же команда с теми же параметрами в этом каталоге."
            }
            Self::WorkspaceWrite => {
                "Для этого агента до перезапуска сессии: разрешённая запись файлов в проекте для этого инструмента."
            }
        }
    }
}
