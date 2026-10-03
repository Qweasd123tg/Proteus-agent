//! Device-local preferences shared by settings, composer and transcript.
use crate::ui_preferences::{load_bool_setting, load_i32_setting};
use leptos::prelude::*;

#[derive(Clone, Copy)]
pub(crate) struct InterfaceSettings {
    pub animations: RwSignal<bool>,
    pub font_size: RwSignal<i32>,
    pub auto_scroll: RwSignal<bool>,
    pub ctrl_enter: RwSignal<bool>,
    pub notifications: RwSignal<bool>,
}
impl InterfaceSettings {
    pub fn new() -> Self {
        Self {
            animations: RwSignal::new(load_bool_setting("proteus.animations", true)),
            font_size: RwSignal::new(load_i32_setting("proteus.fontSize", 16).clamp(12, 22)),
            auto_scroll: RwSignal::new(load_bool_setting("proteus.autoScroll", true)),
            ctrl_enter: RwSignal::new(load_bool_setting("proteus.ctrlEnter", false)),
            notifications: RwSignal::new(load_bool_setting("proteus.notifications", true)),
        }
    }
}
pub(crate) fn settings() -> InterfaceSettings {
    expect_context()
}

pub(crate) fn save_number(key: &str, value: i32) -> Result<(), String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or_else(|| "локальное хранилище недоступно".to_owned())?
        .set_item(key, &value.to_string())
        .map_err(|_| "локальное хранилище недоступно".to_owned())
}
