//! Typed client services shared by built-in and installed UI modules.
use super::{connection::ClientConnection, navigation::AppRouter, state::AppState};
use crate::{interface_settings, types::*};
use leptos::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen(raw_module = "/extensions/web-adapter.js")]
extern "C" {
    #[wasm_bindgen(js_name = configureModules)]
    fn configure(
        preferences: &js_sys::Function,
        composer: &js_sys::Function,
        navigate: &js_sys::Function,
    ) -> js_sys::Function;
    #[wasm_bindgen(js_name = publishModules)]
    fn publish(preferences: &str, composer: &str, url: &str);
    #[wasm_bindgen(js_name = requestSettingsModule)]
    pub(super) fn request_settings_module(id: &str) -> bool;
}

pub(super) fn install(state: AppState, connection: ClientConnection, router: AppRouter) {
    let prefs = interface_settings::settings();
    let preference =
        Closure::<dyn FnMut(String, String) -> String>::new(move |key: String, value: String| {
            write_preference(state, prefs, &key, &value)
                .err()
                .unwrap_or_default()
        });
    let composer =
        Closure::<dyn FnMut(String, String) -> String>::new(move |key: String, value: String| {
            write_composer(state, connection, &key, &value)
                .err()
                .unwrap_or_default()
        });
    let navigate = Closure::<dyn FnMut(String)>::new(move |page: String| {
        if page == "workspace" {
            router.restore_workspace();
        } else {
            router.navigate(if page == "chat" { "/" } else { "/settings" });
        }
    });
    let dispose = configure(
        preference.as_ref().unchecked_ref(),
        composer.as_ref().unchecked_ref(),
        navigate.as_ref().unchecked_ref(),
    );
    let owned = StoredValue::new_local((dispose, preference, composer, navigate));
    on_cleanup(move || {
        owned.with_value(|(dispose, ..)| {
            let _ = dispose.call0(&JsValue::NULL);
        })
    });
    Effect::new(move |_| {
        let preferences = json!({"fontSize":prefs.font_size.get(),"chatWidth":state.view.resize.chat_width.get(),"animations":prefs.animations.get(),"autoScroll":prefs.auto_scroll.get(),"sendMode":if prefs.ctrl_enter.get(){"ctrl-enter"}else{"enter"},"toolCardsCollapsed":state.view.tool_cards_collapsed.get(),"notifications":prefs.notifications.get()});
        let request = state.request;
        let models: Vec<Value> = request
            .model_options
            .get()
            .into_iter()
            .map(|m| json!({"name":m.name,"label":m.label,"hidden":m.hidden}))
            .collect();
        let modes = [
            (PermissionMode::Normal, "С подтверждениями"),
            (PermissionMode::Auto, "Без подтверждений"),
            (PermissionMode::Plan, "Планирование"),
        ]
        .map(|(m, label)| json!({"value":m.label(),"label":label,"description":m.description()}));
        let composer = json!({"model":request.model_name.get(),"models":models,"reasoning":request.reasoning_enabled.get(),"effort":request.effort.get().value(),"effortLabel":request.effort.get().label(),"efforts":request.effort_options.get(),"mode":request.mode.get().label(),"modes":modes});
        publish(
            &preferences.to_string(),
            &composer.to_string(),
            &crate::api::diagnostics_url(state.session.active_session_dir.get().as_deref()),
        );
    });
}

fn write_preference(
    state: AppState,
    prefs: interface_settings::InterfaceSettings,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let value: Value = serde_json::from_str(value).map_err(|e| e.to_string())?;
    if matches!(key, "fontSize" | "chatWidth") {
        let number = value.as_i64().ok_or("Ожидается целое число")?;
        match key {
            "fontSize" if (12..=22).contains(&number) => {
                interface_settings::save_number("proteus.fontSize", number as i32)?;
                prefs.font_size.set(number as i32);
            }
            "chatWidth" if (420..=1600).contains(&number) => {
                state.view.resize.set_width(number as i32)?
            }
            _ => return Err("Значение вне допустимого диапазона".into()),
        }
        return Ok(());
    }
    let (storage, next) = if key == "sendMode" {
        (
            "proteus.ctrlEnter",
            match value.as_str() {
                Some("enter") => false,
                Some("ctrl-enter") => true,
                _ => return Err("Неизвестный режим отправки".into()),
            },
        )
    } else {
        let storage = match key {
            "animations" => "proteus.animations",
            "autoScroll" => "proteus.autoScroll",
            "notifications" => "proteus.notifications",
            "toolCardsCollapsed" => crate::ui_preferences::TOOL_CARDS_COLLAPSED_KEY,
            _ => return Err("Неизвестная настройка".into()),
        };
        (
            storage,
            value.as_bool().ok_or("Ожидается логическое значение")?,
        )
    };
    crate::ui_preferences::try_save_bool_setting(storage, next)?;
    match key {
        "animations" => prefs.animations.set(next),
        "autoScroll" => prefs.auto_scroll.set(next),
        "notifications" => prefs.notifications.set(next),
        "sendMode" => prefs.ctrl_enter.set(next),
        "toolCardsCollapsed" => state.view.set_tool_cards_collapsed.set(next),
        _ => unreachable!(),
    }
    Ok(())
}
fn write_composer(
    state: AppState,
    connection: ClientConnection,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let value: String = serde_json::from_str(value).map_err(|e| e.to_string())?;
    match key {
        "model"
            if state
                .request
                .model_options
                .with_untracked(|v| v.iter().any(|m| m.name == value)) =>
        {
            connection.actions.set_model_name(value)
        }
        "effort"
            if state
                .request
                .effort_options
                .with_untracked(|v| v.contains(&value)) =>
        {
            connection
                .actions
                .set_reasoning_effort(ReasoningEffort::from_value(&value))
        }
        "mode" if ["normal", "auto", "plan"].contains(&value.as_str()) => connection
            .actions
            .set_permission_mode(PermissionMode::from_value(&value)),
        _ => return Err("Недоступный выбор".into()),
    }
    Ok(())
}
