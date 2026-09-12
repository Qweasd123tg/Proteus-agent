pub(crate) use proteus_contracts::app_protocol::{AppServerEvent, StdioOutput};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TransportStatus {
    Connecting,
    Connected,
    /// EventSource оборвался и сам ретраит: ошибку не показываем,
    /// пока не истечёт грейс-период (см. events.rs).
    Reconnecting,
    Error(String),
    Shutdown,
}

impl TransportStatus {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Connecting => "подключение".to_owned(),
            Self::Connected => "подключено".to_owned(),
            Self::Reconnecting => "переподключение".to_owned(),
            Self::Error(message) => format!("ошибка: {message}"),
            Self::Shutdown => "остановлено".to_owned(),
        }
    }
}
