//! Last successful manual selection is a client preference for newly created chats.
use crate::{
    api::{get_json, post_json, session_path},
    types::{SetModelRequest, SetReasoningEffortRequest, StdioOutput},
};
use proteus_contracts::app_protocol::config::ConfigSummary;
use serde::{Deserialize, Serialize};
const KEY: &str = "proteus.model.last-selection";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    pub model: String,
    pub effort: Option<String>,
}
fn storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or_else(|| "Локальное хранилище недоступно".to_owned())
}
pub(crate) async fn read_current(session: &str) -> Result<Selection, String> {
    let config = get_json::<ConfigSummary>(&session_path("/config", session)).await?;
    Ok(Selection {
        model: config
            .model
            .ok_or_else(|| "В этой сборке модель не выбрана".to_owned())?
            .name,
        effort: if config.reasoning.enabled {
            config.reasoning.effort
        } else {
            Some("none".to_owned())
        },
    })
}
pub(crate) fn remember(selection: &Selection) -> Result<(), String> {
    storage()?
        .set_item(
            KEY,
            &serde_json::to_string(selection).map_err(|e| e.to_string())?,
        )
        .map_err(|_| "Не удалось сохранить модель и effort".to_owned())
}
fn checked(output: StdioOutput) -> Result<(), String> {
    match output {
        StdioOutput::Response { ok: true, .. } => Ok(()),
        StdioOutput::Response { error, .. } => {
            Err(error.unwrap_or_else(|| "Настройка недоступна".to_owned()))
        }
        _ => Err("Неожиданный ответ настройки модели".to_owned()),
    }
}
pub(crate) async fn restore(session: &str) -> Result<(), String> {
    let Some(value) = storage()?
        .get_item(KEY)
        .map_err(|_| "Не удалось прочитать сохранённую модель")?
    else {
        return Ok(());
    };
    let config = get_json::<ConfigSummary>(&session_path("/config", session)).await?;
    if config.model.is_none() {
        return Ok(());
    }
    let selection: Selection =
        serde_json::from_str(&value).map_err(|e| format!("Некорректные настройки модели: {e}"))?;
    checked(
        post_json(
            "/model",
            &SetModelRequest {
                id: Some("restore-last-model".into()),
                model: selection.model,
                session_dir: session.into(),
            },
        )
        .await?,
    )?;
    checked(
        post_json(
            "/effort",
            &SetReasoningEffortRequest {
                id: Some("restore-last-effort".into()),
                effort: selection.effort,
                session_dir: session.into(),
            },
        )
        .await?,
    )
}
