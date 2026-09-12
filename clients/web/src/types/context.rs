use serde::{Deserialize, Serialize};

/// Заполнение контекстного окна по данным события `TokenUsageUpdated`.
/// Последний валидный снимок сохраняется клиентом, чтобы бублик сразу
/// восстанавливался при возврате в чат.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ContextUsage {
    pub(crate) used_tokens: u32,
    pub(crate) max_tokens: u32,
    /// Порог токенов, на котором сервер запускает автокомпакт. `None`, если
    /// автокомпакт не настроен — тогда метка на бублике не рисуется.
    pub(crate) compaction_trigger_tokens: Option<u32>,
}

pub(crate) use proteus_contracts::app_protocol::{
    AppContextCompactionSnapshot as ContextCompactionSnapshot,
    AppContextMapSnapshot as ContextMapSnapshot, AppContextUsageCategory as ContextUsageCategory,
    AppContextUsageSnapshot as ContextUsageSnapshot,
};
pub(crate) use proteus_contracts::domain::HistoryCompactionReport as ContextCompactionReport;
#[cfg(test)]
pub(crate) use proteus_contracts::model_standard::TokenUsage as ContextActualUsage;
