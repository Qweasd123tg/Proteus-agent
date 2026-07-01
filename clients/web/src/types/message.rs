use super::ToolActivity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MessageRole {
    User,
    Assistant,
    System,
    /// Поток reasoning-summary модели (OpenAI o-series). Рендерится
    /// отдельным сворачиваемым блоком, не как обычное сообщение.
    Reasoning,
}

impl MessageRole {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::User => "Вы",
            Self::Assistant => "Proteus",
            Self::System => "Система",
            Self::Reasoning => "Размышления",
        }
    }

    pub(crate) fn message_class(&self) -> &'static str {
        match self {
            Self::User => "message user-message",
            Self::Assistant => "message assistant-message",
            Self::System | Self::Reasoning => "message system-message",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Message {
    pub(crate) id: u64,
    pub(crate) version: u64,
    pub(crate) role: MessageRole,
    pub(crate) text: String,
    pub(crate) tool: Option<ToolActivity>,
    pub(crate) streaming: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToastMessage {
    pub(crate) id: u64,
    pub(crate) text: String,
}
