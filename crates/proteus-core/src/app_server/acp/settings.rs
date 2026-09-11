//! Session selectors project the same provider-owned selection as HTTP/stdio.
use agent_client_protocol::{Client, ConnectionTo, Result, schema::v1::*};

use super::{Session, input, internal, invalid};
use crate::app_server::{AppServerHandle, model_selection::ModelSelection};

const DEFAULT_EFFORT: &str = "_default";

fn effort_id(effort: &str) -> String {
    format!("effort:{effort}")
}

pub(super) async fn options(server: &AppServerHandle) -> Result<Vec<SessionConfigOption>> {
    let modes = input::modes(server.permission_mode().await)?;
    Ok(render(server.model_selection().await, modes))
}

fn render(selection: ModelSelection, modes: SessionModeState) -> Vec<SessionConfigOption> {
    let mut options = vec![
        SessionConfigOption::select(
            "mode",
            "Permissions",
            modes.current_mode_id.0.to_string(),
            modes
                .available_modes
                .into_iter()
                .map(|mode| {
                    SessionConfigSelectOption::new(mode.id.0.to_string(), mode.name)
                        .description(mode.description)
                })
                .collect::<Vec<_>>(),
        )
        .category(SessionConfigOptionCategory::Mode),
    ];
    if let Some(error) = selection.summary.error {
        // An unavailable catalog is not permission to invent model choices.
        eprintln!("ACP model catalog unavailable: {error}");
        return options;
    }
    let models = selection
        .summary
        .models
        .into_iter()
        .filter(|model| !model.hidden || model.name == selection.active.model)
        .map(|model| {
            SessionConfigSelectOption::new(model.name, model.label).description(model.description)
        })
        .collect::<Vec<_>>();
    if !models.is_empty() {
        options.push(
            SessionConfigOption::select("model", "Model", selection.active.model, models)
                .category(SessionConfigOptionCategory::Model),
        );
    }
    if !selection.summary.efforts.is_empty() {
        let mut efforts = vec![
            SessionConfigSelectOption::new(DEFAULT_EFFORT, "Default")
                .description("Let the provider choose the reasoning effort"),
        ];
        efforts.extend(
            selection
                .summary
                .efforts
                .into_iter()
                .map(|effort| SessionConfigSelectOption::new(effort_id(&effort), effort)),
        );
        options.push(
            SessionConfigOption::select(
                "reasoning_effort",
                "Reasoning",
                selection
                    .reasoning
                    .effort
                    .as_deref()
                    .map(effort_id)
                    .unwrap_or_else(|| DEFAULT_EFFORT.to_owned()),
                efforts,
            )
            .category(SessionConfigOptionCategory::ThoughtLevel),
        );
    }
    options
}

pub(super) async fn set(
    session: Session,
    request: SetSessionConfigOptionRequest,
    cx: ConnectionTo<Client>,
) -> Result<SetSessionConfigOptionResponse> {
    // Hold through response projection, just like a prompt. Runtime settings
    // cannot change halfway through the protocol's active prompt lifetime.
    let _lease = session.reserve()?;
    let value = request
        .value
        .as_value_id()
        .ok_or_else(|| invalid("configuration selectors require a string value"))?;
    let value = value.0.as_ref();
    let available = options(&session.server).await?;
    let option = available
        .iter()
        .find(|option| option.id == request.config_id)
        .ok_or_else(|| invalid("unknown or unavailable configId"))?;
    let SessionConfigKind::Select(select) = &option.kind else {
        return Err(invalid("unsupported configuration option type"));
    };
    let SessionConfigSelectOptions::Ungrouped(choices) = &select.options else {
        return Err(invalid("unsupported configuration option group"));
    };
    if !choices
        .iter()
        .any(|choice| choice.value.0.as_ref() == value)
    {
        return Err(invalid("value is absent from this configuration option"));
    }
    match request.config_id.0.as_ref() {
        "model" => session
            .server
            .set_model_name(value.to_owned())
            .await
            .map_err(internal)?,
        "reasoning_effort" => session
            .server
            .set_reasoning_effort(value.strip_prefix("effort:").map(str::to_owned))
            .await
            .map_err(internal)?,
        "mode" => {
            session
                .server
                .set_permission_mode(input::permission_mode(&SessionModeId::new(value))?)
                .await;
            cx.send_notification(SessionNotification::new(
                request.session_id.clone(),
                SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(value.to_owned())),
            ))?;
        }
        _ => return Err(invalid("unknown configId")),
    }
    let options = options(&session.server).await?;
    notify(&cx, request.session_id, options.clone())?;
    Ok(SetSessionConfigOptionResponse::new(options))
}

pub(super) fn notify(
    cx: &ConnectionTo<Client>,
    session: SessionId,
    options: Vec<SessionConfigOption>,
) -> Result<()> {
    cx.send_notification(SessionNotification::new(
        session,
        SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(options)),
    ))
}

#[cfg(test)]
mod tests;
