pub(crate) use proteus_contracts::{
    app_protocol::http::{
        ApprovalRequest as ResolveApprovalRequest, CancelRequest, DeleteQueuedMessageRequest,
        DeleteSessionRequest, EditQueuedMessageRequest, ResumeSessionRequest, SendRequest,
        SetModelRequest, SetPermissionModeRequest, SetReasoningEffortRequest,
        UserInputRequest as UserInputSubmitRequest,
    },
    contracts::{
        UserInputAnswer as UserInputAnswerBody, UserInputResponse as UserInputResponseBody,
    },
};
