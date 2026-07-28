use serde::Deserialize;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct RequestOriginInfo {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputOption {
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) preview: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputQuestion {
    pub(crate) id: String,
    pub(crate) header: String,
    pub(crate) question: String,
    pub(crate) is_other: bool,
    pub(crate) is_secret: bool,
    pub(crate) multi_select: bool,
    pub(crate) options: Vec<UserInputOption>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputRequestInfo {
    pub(crate) request_id: String,
    pub(crate) cwd: String,
    pub(crate) title: Option<String>,
    pub(crate) questions: Vec<UserInputQuestion>,
    /// Атрибуция запроса: thread/turn + метка источника (роль субагента).
    pub(crate) origin: Option<RequestOriginInfo>,
    /// Порядковый номер в очереди user inputs.
    pub(crate) seq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct PendingControlPlaneInfo {
    pub(crate) user_inputs: Vec<UserInputRequestInfo>,
    pub(crate) queued_user_messages: Vec<QueuedPromptInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct QueuedPromptInfo {
    pub(crate) message_id: String,
    pub(crate) text: String,
}
